//! Project-owned x86-64 encoder for the native subset and bootstrap runtime.
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reg(u8);
impl Reg {
    pub fn number(self) -> u8 {
        self.0
    }
    pub fn new(n: u8) -> Result<Self, String> {
        if n < 16 {
            Ok(Self(n))
        } else {
            Err("register must be in 0..16".into())
        }
    }
    pub const RAX: Self = Self(0);
    pub const RCX: Self = Self(1);
    pub const RDX: Self = Self(2);
    pub const RSP: Self = Self(4);
    pub const RBP: Self = Self(5);
    fn low(self) -> u8 {
        self.0 & 7
    }
    fn high(self) -> u8 {
        self.0 >> 3
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Dword,
    Qword,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    One,
    Two,
    Four,
    Eight,
}
impl Scale {
    fn bits(self) -> u8 {
        match self {
            Self::One => 0,
            Self::Two => 1,
            Self::Four => 2,
            Self::Eight => 3,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Address {
    pub base: Reg,
    pub index: Option<(Reg, Scale)>,
    pub displacement: i32,
}
impl Address {
    pub fn base(base: Reg, displacement: i32) -> Self {
        Self {
            base,
            index: None,
            displacement,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Binary {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Compare,
    Test,
}
#[derive(Debug, Clone, Copy)]
pub enum Immediate {
    Add,
    Sub,
    Compare,
}
#[derive(Debug, Clone, Copy)]
pub enum Condition {
    Overflow,
    NotOverflow,
    Below,
    AboveEqual,
    Equal,
    NotEqual,
    BelowEqual,
    Above,
    Sign,
    NotSign,
    Less,
    GreaterEqual,
    LessEqual,
    Greater,
}
impl Condition {
    fn code(self) -> u8 {
        match self {
            Self::Overflow => 0,
            Self::NotOverflow => 1,
            Self::Below => 2,
            Self::AboveEqual => 3,
            Self::Equal => 4,
            Self::NotEqual => 5,
            Self::BelowEqual => 6,
            Self::Above => 7,
            Self::Sign => 8,
            Self::NotSign => 9,
            Self::Less => 12,
            Self::GreaterEqual => 13,
            Self::LessEqual => 14,
            Self::Greater => 15,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(pub usize);
#[derive(Debug, Clone)]
pub enum Instruction {
    MovImm {
        width: Width,
        dst: Reg,
        value: u64,
    },
    Mov {
        width: Width,
        dst: Reg,
        src: Reg,
    },
    Load {
        width: Width,
        dst: Reg,
        address: Address,
    },
    Store {
        width: Width,
        address: Address,
        src: Reg,
    },
    Lea {
        dst: Reg,
        address: Address,
    },
    LoadByte {
        dst: Reg,
        address: Address,
    },
    StoreByte {
        address: Address,
        src: Reg,
    },
    Binary {
        width: Width,
        op: Binary,
        dst: Reg,
        src: Reg,
    },
    Multiply {
        width: Width,
        dst: Reg,
        src: Reg,
    },
    Negate {
        width: Width,
        reg: Reg,
    },
    SignExtend(Width),
    Divide {
        width: Width,
        divisor: Reg,
    },
    Set {
        condition: Condition,
        dst: Reg,
    },
    Immediate {
        width: Width,
        op: Immediate,
        reg: Reg,
        value: i32,
    },
    Push(Reg),
    Pop(Reg),
    CallReg(Reg),
    Bind(Label),
    Jump(Label),
    Branch {
        condition: Condition,
        label: Label,
    },
    Call(Label),
    CallImport(Symbol),
    LeaSymbol {
        dst: Reg,
        symbol: Symbol,
    },
    Ret,
    Nop,
}
#[derive(Debug, Clone)]
pub struct Listing {
    pub offset: usize,
    pub length: usize,
    pub instruction: Instruction,
}
#[derive(Debug, Clone)]
pub struct ExternalFixup {
    pub displacement_offset: usize,
    pub next_instruction: usize,
    pub symbol: Symbol,
}
#[derive(Debug, Clone)]
pub struct MachineFragment {
    pub bytes: Vec<u8>,
    pub listing: Vec<Listing>,
    pub external_fixups: Vec<ExternalFixup>,
}
/// Calculate rel32 using a wide intermediate so invalid ranges never wrap.
pub fn relative32(target: u64, next: u64) -> Result<i32, String> {
    i32::try_from(i128::from(target) - i128::from(next))
        .map_err(|_| "relative displacement exceeds signed32 range".into())
}
impl MachineFragment {
    pub fn resolve(
        &self,
        code_rva: u32,
        symbols: &HashMap<Symbol, u32>,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = self.bytes.clone();
        for fix in &self.external_fixups {
            let target = *symbols
                .get(&fix.symbol)
                .ok_or("unresolved machine symbol")?;
            let next = u64::from(code_rva)
                .checked_add(
                    u64::try_from(fix.next_instruction).map_err(|_| "code offset overflow")?,
                )
                .ok_or("code address overflow")?;
            let displacement = relative32(target as u64, next)?;
            let end = fix
                .displacement_offset
                .checked_add(4)
                .ok_or("fixup offset overflow")?;
            if fix.next_instruction != end || end > self.bytes.len() {
                return Err("inconsistent external fixup boundary".into());
            }
            let slot = bytes
                .get_mut(fix.displacement_offset..end)
                .ok_or("fixup outside fragment")?;
            slot.copy_from_slice(&displacement.to_le_bytes());
        }
        Ok(bytes)
    }
    pub fn dump(&self) -> String {
        let mut text = String::new();
        for line in &self.listing {
            text.push_str(&format!(
                "{:08x} +{:02} {:?}\n",
                line.offset, line.length, line.instruction
            ));
        }
        text
    }
}
fn rex(b: &mut Vec<u8>, w: bool, r: u8, x: u8, base: u8, force: bool) {
    let value = 0x40 | ((w as u8) << 3) | (r << 2) | (x << 1) | base;
    if value != 0x40 || force {
        b.push(value);
    }
}
fn rr(b: &mut Vec<u8>, width: Width, opcode: &[u8], reg: Reg, rm: Reg) {
    rex(b, width == Width::Qword, reg.high(), 0, rm.high(), false);
    b.extend(opcode);
    b.push(0xc0 | (reg.low() << 3) | rm.low());
}
fn memory(
    b: &mut Vec<u8>,
    width: Width,
    opcode: &[u8],
    reg: Reg,
    a: Address,
    force: bool,
) -> Result<(), String> {
    if a.index.is_some_and(|(r, _)| r == Reg::RSP) {
        return Err("RSP cannot be an index register".into());
    }
    let mode = if a.displacement == 0 && a.base.low() != 5 {
        0
    } else if i8::try_from(a.displacement).is_ok() {
        1
    } else {
        2
    };
    let sib = a.index.is_some() || a.base.low() == 4;
    rex(
        b,
        width == Width::Qword,
        reg.high(),
        a.index.map_or(0, |(r, _)| r.high()),
        a.base.high(),
        force,
    );
    b.extend(opcode);
    b.push((mode << 6) | (reg.low() << 3) | if sib { 4 } else { a.base.low() });
    if sib {
        let (index, scale) = a.index.map_or((4, 0), |(r, s)| (r.low(), s.bits()));
        b.push((scale << 6) | (index << 3) | a.base.low());
    }
    if mode == 1 {
        b.push(a.displacement as i8 as u8);
    } else if mode == 2 {
        b.extend(a.displacement.to_le_bytes());
    }
    Ok(())
}
/// Encode one complete fragment transactionally; no output is returned on error.
pub fn encode(instructions: &[Instruction]) -> Result<MachineFragment, String> {
    let mut bytes = Vec::new();
    let mut listing = Vec::new();
    let mut labels = HashMap::new();
    let mut local = Vec::new();
    let mut external_fixups = Vec::new();
    for instruction in instructions {
        let start = bytes.len();
        match *instruction {
            Instruction::MovImm { width, dst, value } => {
                if width == Width::Dword && value > u32::MAX as u64 {
                    return Err("immediate does not fit 32 bits".into());
                }
                rex(&mut bytes, width == Width::Qword, 0, 0, dst.high(), false);
                bytes.push(0xb8 + dst.low());
                if width == Width::Qword {
                    bytes.extend(value.to_le_bytes());
                } else {
                    bytes.extend((value as u32).to_le_bytes());
                }
            }
            Instruction::Mov { width, dst, src } => rr(&mut bytes, width, &[0x89], src, dst),
            Instruction::Load {
                width,
                dst,
                address,
            } => memory(&mut bytes, width, &[0x8b], dst, address, false)?,
            Instruction::Store {
                width,
                address,
                src,
            } => memory(&mut bytes, width, &[0x89], src, address, false)?,
            Instruction::Lea { dst, address } => {
                memory(&mut bytes, Width::Qword, &[0x8d], dst, address, false)?
            }
            Instruction::LoadByte { dst, address } => {
                memory(&mut bytes, Width::Dword, &[0x0f, 0xb6], dst, address, false)?
            }
            Instruction::StoreByte { address, src } => memory(
                &mut bytes,
                Width::Dword,
                &[0x88],
                src,
                address,
                src.low() >= 4,
            )?,
            Instruction::Binary {
                width,
                op,
                dst,
                src,
            } => rr(
                &mut bytes,
                width,
                &[match op {
                    Binary::Add => 0x01,
                    Binary::Sub => 0x29,
                    Binary::And => 0x21,
                    Binary::Or => 0x09,
                    Binary::Xor => 0x31,
                    Binary::Compare => 0x39,
                    Binary::Test => 0x85,
                }],
                src,
                dst,
            ),
            Instruction::Multiply { width, dst, src } => {
                rr(&mut bytes, width, &[0x0f, 0xaf], dst, src)
            }
            Instruction::Negate { width, reg } => {
                rex(&mut bytes, width == Width::Qword, 0, 0, reg.high(), false);
                bytes.extend([0xf7, 0xd8 | reg.low()]);
            }
            Instruction::SignExtend(width) => {
                rex(&mut bytes, width == Width::Qword, 0, 0, 0, false);
                bytes.push(0x99);
            }
            Instruction::Divide { width, divisor } => {
                rex(
                    &mut bytes,
                    width == Width::Qword,
                    0,
                    0,
                    divisor.high(),
                    false,
                );
                bytes.extend([0xf7, 0xf8 | divisor.low()]);
            }
            Instruction::Set { condition, dst } => {
                rex(&mut bytes, false, 0, 0, dst.high(), dst.low() >= 4);
                bytes.extend([0x0f, 0x90 + condition.code(), 0xc0 | dst.low()]);
                rex(&mut bytes, false, dst.high(), 0, dst.high(), dst.low() >= 4);
                bytes.extend([0x0f, 0xb6, 0xc0 | (dst.low() << 3) | dst.low()]);
            }
            Instruction::Immediate {
                width,
                op,
                reg,
                value,
            } => {
                rex(&mut bytes, width == Width::Qword, 0, 0, reg.high(), false);
                let group = match op {
                    Immediate::Add => 0,
                    Immediate::Sub => 5,
                    Immediate::Compare => 7,
                };
                bytes.extend([0x81, 0xc0 | (group << 3) | reg.low()]);
                bytes.extend(value.to_le_bytes());
            }
            Instruction::Push(reg) | Instruction::Pop(reg) => {
                rex(&mut bytes, false, 0, 0, reg.high(), false);
                bytes.push(
                    if matches!(instruction, Instruction::Push(_)) {
                        0x50
                    } else {
                        0x58
                    } + reg.low(),
                );
            }
            Instruction::CallReg(reg) => {
                rex(&mut bytes, false, 0, 0, reg.high(), false);
                bytes.extend([0xff, 0xd0 | reg.low()]);
            }
            Instruction::Bind(label) => {
                if labels.insert(label, bytes.len()).is_some() {
                    return Err("duplicate machine label".into());
                }
            }
            Instruction::Jump(label) | Instruction::Call(label) => {
                bytes.push(if matches!(instruction, Instruction::Jump(_)) {
                    0xe9
                } else {
                    0xe8
                });
                let offset = bytes.len();
                bytes.extend([0; 4]);
                local.push((offset, bytes.len(), label));
            }
            Instruction::Branch { condition, label } => {
                bytes.extend([0x0f, 0x80 + condition.code()]);
                let offset = bytes.len();
                bytes.extend([0; 4]);
                local.push((offset, bytes.len(), label));
            }
            Instruction::CallImport(symbol) => {
                bytes.extend([0xff, 0x15]);
                let offset = bytes.len();
                bytes.extend([0; 4]);
                external_fixups.push(ExternalFixup {
                    displacement_offset: offset,
                    next_instruction: bytes.len(),
                    symbol,
                });
            }
            Instruction::LeaSymbol { dst, symbol } => {
                rex(&mut bytes, true, dst.high(), 0, 0, false);
                bytes.extend([0x8d, 0x05 | (dst.low() << 3)]);
                let offset = bytes.len();
                bytes.extend([0; 4]);
                external_fixups.push(ExternalFixup {
                    displacement_offset: offset,
                    next_instruction: bytes.len(),
                    symbol,
                });
            }
            Instruction::Ret => bytes.push(0xc3),
            Instruction::Nop => bytes.push(0x90),
        }
        listing.push(Listing {
            offset: start,
            length: bytes.len() - start,
            instruction: instruction.clone(),
        });
    }
    for (at, next, label) in local {
        let target = *labels.get(&label).ok_or("undefined machine label")?;
        let displacement = relative32(target as u64, next as u64)?;
        bytes[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
    }
    Ok(MachineFragment {
        bytes,
        listing,
        external_fixups,
    })
}
