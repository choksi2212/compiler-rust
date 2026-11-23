//! Windows x64 integer/pointer ABI, stack frames, page probes and unwind data.
use crate::x64::{Address, Condition, Immediate, Instruction, Label, Reg, Width, encode};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentLocation {
    Register(Reg),
    Stack(u32),
}
pub fn scalar_argument(index: usize) -> Result<ArgumentLocation, String> {
    if index < 4 {
        Ok(ArgumentLocation::Register(
            [Reg::RCX, Reg::RDX, Reg::new(8)?, Reg::new(9)?][index],
        ))
    } else {
        let offset = index
            .checked_sub(4)
            .and_then(|n| n.checked_mul(8))
            .and_then(|n| n.checked_add(32))
            .ok_or("argument offset overflow")?;
        Ok(ArgumentLocation::Stack(
            u32::try_from(offset).map_err(|_| "argument offset exceeds32bits")?,
        ))
    }
}
#[derive(Debug, Clone, Default)]
pub struct FrameRequest {
    pub has_calls: bool,
    pub max_call_arguments: usize,
    pub locals: Vec<(u32, u32)>,
    pub temporaries: usize,
    pub initialization_flags: usize,
    pub saved: Vec<Reg>,
}
#[derive(Debug, Clone)]
pub struct FramePlan {
    allocation: u32,
    outgoing_size: u32,
    local_offsets: Vec<u32>,
    temporary_offsets: Vec<u32>,
    flag_offsets: Vec<u32>,
    saved: Vec<Reg>,
}
#[derive(Debug, Clone)]
pub struct Prologue {
    pub instructions: Vec<Instruction>,
    pub unwind: Vec<u8>,
    pub byte_length: u8,
}
fn aligned(n: u32, a: u32) -> Result<u32, String> {
    n.checked_add(a - 1)
        .map(|n| n & !(a - 1))
        .ok_or("frame alignment overflow".into())
}
impl FramePlan {
    pub fn allocation(&self) -> u32 {
        self.allocation
    }
    pub fn outgoing_size(&self) -> u32 {
        self.outgoing_size
    }
    pub fn local_offsets(&self) -> &[u32] {
        &self.local_offsets
    }
    pub fn temporary_offsets(&self) -> &[u32] {
        &self.temporary_offsets
    }
    pub fn flag_offsets(&self) -> &[u32] {
        &self.flag_offsets
    }
    pub fn saved_registers(&self) -> &[Reg] {
        &self.saved
    }

    pub fn new(request: &FrameRequest) -> Result<Self, String> {
        if !request.has_calls && request.max_call_arguments != 0 {
            return Err("leaf frame cannot reserve call arguments".into());
        }
        let mut seen = std::collections::HashSet::new();
        for r in &request.saved {
            if ![3, 5, 6, 7, 12, 13, 14, 15].contains(&r.number()) || !seen.insert(*r) {
                return Err("invalid/duplicate nonvolatile save register".into());
            }
        }
        let outgoing_size = if request.has_calls {
            let args = request
                .max_call_arguments
                .max(4)
                .checked_mul(8)
                .ok_or("outgoing arguments overflow")?;
            u32::try_from(args).map_err(|_| "outgoing arguments exceed32bits")?
        } else {
            0
        };
        let mut size = outgoing_size;
        let mut local_offsets = vec![];
        for (bytes, alignment) in &request.locals {
            if *bytes == 0 || !alignment.is_power_of_two() || *alignment > 16 {
                return Err("invalid local size/alignment".into());
            }
            size = aligned(size, *alignment)?;
            local_offsets.push(size);
            size = size.checked_add(*bytes).ok_or("local frame overflow")?;
        }
        let temp_count =
            u32::try_from(request.temporaries).map_err(|_| "temporary count overflow")?;
        size = aligned(size, 8)?;
        let temp_start = size;
        size = size
            .checked_add(temp_count.checked_mul(8).ok_or("temporary size overflow")?)
            .ok_or("frame overflow")?;
        let flag_count = u32::try_from(request.initialization_flags)
            .map_err(|_| "initialization flag count overflow")?;
        let flag_start = size;
        size = size.checked_add(flag_count).ok_or("flag frame overflow")?;
        let desired = if request.saved.len().is_multiple_of(2) {
            8
        } else {
            0
        };
        size = aligned(size, 8)?;
        if size % 16 != desired {
            size = size.checked_add(8).ok_or("frame size overflow")?;
        }
        if size > 1024 * 1024 {
            return Err("initial frame limit is1MiB".into());
        }
        let temporary_offsets = (0..temp_count).map(|n| temp_start + n * 8).collect();
        let flag_offsets = (0..flag_count).map(|n| flag_start + n).collect();
        Ok(Self {
            allocation: size,
            outgoing_size,
            local_offsets,
            temporary_offsets,
            flag_offsets,
            saved: request.saved.clone(),
        })
    }
    pub fn incoming_stack_offset(&self, index: usize) -> Result<u32, String> {
        let ArgumentLocation::Stack(at) = scalar_argument(index)? else {
            return Err("argument is register-passed".into());
        };
        self.allocation
            .checked_add(self.saved.len() as u32 * 8)
            .and_then(|v| v.checked_add(8))
            .and_then(|v| v.checked_add(at))
            .ok_or("incoming argument offset overflow".into())
    }
    /// Address of a positional argument's home relative to the completed frame.
    /// Positions0..3 name the caller's shadow space; later positions are stack args.
    pub fn incoming_home_offset(&self, index: usize) -> Result<u32, String> {
        let position = u32::try_from(index).map_err(|_| "argument position overflow")?;
        self.allocation
            .checked_add(self.saved.len() as u32 * 8)
            .and_then(|v| v.checked_add(8))
            .and_then(|v| position.checked_mul(8).and_then(|at| v.checked_add(at)))
            .ok_or("incoming argument home overflow".into())
    }
    pub fn prologue(&self, probe_label: Label) -> Result<Prologue, String> {
        let mut instructions = vec![];
        for r in &self.saved {
            instructions.push(Instruction::Push(*r));
        }
        if self.allocation > 4096 {
            let r10 = Reg::new(10)?;
            let r11 = Reg::new(11)?;
            instructions.extend([
                Instruction::Mov {
                    width: Width::Qword,
                    dst: r11,
                    src: Reg::RSP,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: r10,
                    value: self.allocation as u64,
                },
                Instruction::Bind(probe_label),
                Instruction::Immediate {
                    width: Width::Qword,
                    op: Immediate::Sub,
                    reg: r11,
                    value: 4096,
                },
                Instruction::LoadByte {
                    dst: Reg::RAX,
                    address: Address::base(r11, 0),
                },
                Instruction::Immediate {
                    width: Width::Qword,
                    op: Immediate::Sub,
                    reg: r10,
                    value: 4096,
                },
                Instruction::Immediate {
                    width: Width::Qword,
                    op: Immediate::Compare,
                    reg: r10,
                    value: 4096,
                },
                Instruction::Branch {
                    condition: Condition::AboveEqual,
                    label: probe_label,
                },
                Instruction::LoadByte {
                    dst: Reg::RAX,
                    address: Address::base(Reg::RSP, -(self.allocation as i32)),
                },
            ]);
        }
        if self.allocation != 0 {
            instructions.push(Instruction::Immediate {
                width: Width::Qword,
                op: Immediate::Sub,
                reg: Reg::RSP,
                value: self.allocation as i32,
            });
        }
        let fragment = encode(&instructions)?;
        let byte_length = u8::try_from(fragment.bytes.len())
            .map_err(|_| "prologue exceeds unwind length field")?;
        let mut codes = vec![];
        if self.allocation != 0 {
            codes.push(byte_length);
            if self.allocation <= 128 {
                codes.push(2 | (((self.allocation - 8) / 8) as u8) << 4);
            } else if self.allocation / 8 <= u16::MAX as u32 {
                codes.push(1);
                codes.extend(((self.allocation / 8) as u16).to_le_bytes());
            } else {
                codes.push(0x11);
                codes.extend(self.allocation.to_le_bytes());
            }
        }
        for (i, r) in self.saved.iter().enumerate().rev() {
            let row = &fragment.listing[i];
            codes.push(u8::try_from(row.offset + row.length).map_err(|_| "save offset overflow")?);
            codes.push(r.number() << 4);
        }
        let slots = u8::try_from(codes.len() / 2).map_err(|_| "too many unwind slots")?;
        let mut unwind = vec![1, byte_length, slots, 0];
        unwind.extend(codes);
        while unwind.len() % 4 != 0 {
            unwind.push(0);
        }
        Ok(Prologue {
            instructions,
            unwind,
            byte_length,
        })
    }
    pub fn epilogue(&self) -> Vec<Instruction> {
        let mut code = vec![];
        if self.allocation != 0 {
            code.push(Instruction::Immediate {
                width: Width::Qword,
                op: Immediate::Add,
                reg: Reg::RSP,
                value: self.allocation as i32,
            });
        }
        for r in self.saved.iter().rev() {
            code.push(Instruction::Pop(*r));
        }
        code.push(Instruction::Ret);
        code
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct RuntimeFunction {
    pub begin: u32,
    pub end: u32,
    pub unwind: u32,
}
/// Build sorted PE runtime-function records. Unwind bytes are emitted separately.
pub fn function_table(records: &[RuntimeFunction]) -> Result<Vec<u8>, String> {
    let mut records = records.to_vec();
    records.sort_by_key(|r| r.begin);
    let mut bytes = vec![];
    let mut previous = 0;
    for r in records {
        if r.begin >= r.end || r.begin < previous || r.unwind % 4 != 0 {
            return Err("invalid/overlapping runtime function record".into());
        }
        bytes.extend(r.begin.to_le_bytes());
        bytes.extend(r.end.to_le_bytes());
        bytes.extend(r.unwind.to_le_bytes());
        previous = r.end;
    }
    Ok(bytes)
}
#[derive(Debug, Clone)]
pub struct FunctionUnwind {
    pub begin_offset: u32,
    pub end_offset: u32,
    pub unwind: Vec<u8>,
}
/// Check the version1 push/allocation records emitted by this module.
pub fn validate_unwind(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 4 || bytes[0] != 1 || bytes[3] != 0 {
        return Err("unsupported unwind header".into());
    }
    let slots = bytes[2] as usize;
    let end = 4 + slots * 2;
    if bytes.len() != end.div_ceil(4) * 4 || !bytes[end..].iter().all(|b| *b == 0) {
        return Err("invalid unwind length/padding".into());
    }
    let mut slot = 0;
    let mut previous = bytes[1];
    while slot < slots {
        let at = 4 + slot * 2;
        let offset = bytes[at];
        if offset == 0 || offset > previous {
            return Err("invalid unwind code order".into());
        }
        previous = offset;
        let opcode = bytes[at + 1] & 15;
        let info = bytes[at + 1] >> 4;
        let extra = match opcode {
            0 => {
                if ![3, 5, 6, 7, 12, 13, 14, 15].contains(&info) {
                    return Err("invalid unwind save register".into());
                }
                0
            }
            1 => match info {
                0 => 1,
                1 => 2,
                _ => return Err("invalid large unwind allocation".into()),
            },
            2 => 0,
            _ => return Err("unsupported unwind operation".into()),
        };
        if slot + extra >= slots {
            return Err("truncated unwind allocation".into());
        }
        if opcode == 1 {
            let size = if info == 0 {
                u16::from_le_bytes(bytes[at + 2..at + 4].try_into().unwrap()) as u32 * 8
            } else {
                u32::from_le_bytes(bytes[at + 2..at + 6].try_into().unwrap())
            };
            if size == 0 || size % 8 != 0 {
                return Err("invalid unwind allocation size".into());
            }
        }
        slot += 1 + extra;
    }
    Ok(())
}
/// Add real .xdata/.pdata metadata transactionally. The provisional layout has
/// the final section sizes; image-relative records are filled from that layout.
pub fn with_unwind(
    spec: &crate::pe::ImageSpec,
    functions: &[FunctionUnwind],
) -> Result<crate::pe::ImageSpec, String> {
    use crate::pe::{Access, DataSection, DirectoryRef};
    if functions.is_empty() || spec.exception_directory.is_some() {
        return Err("expected new nonempty unwind metadata".into());
    }
    let mut unwind_bytes = vec![];
    let mut offsets = vec![];
    for function in functions {
        validate_unwind(&function.unwind)?;
        if function.begin_offset >= function.end_offset
            || function.end_offset as usize > spec.text.bytes.len()
            || function.unwind[1] as u32 > function.end_offset - function.begin_offset
        {
            return Err("function/unwind range outside text".into());
        }
        while unwind_bytes.len() % 4 != 0 {
            unwind_bytes.push(0);
        }
        offsets.push(u32::try_from(unwind_bytes.len()).map_err(|_| "unwind size overflow")?);
        unwind_bytes.extend(&function.unwind);
    }
    let table_size = functions
        .len()
        .checked_mul(12)
        .ok_or("function table size overflow")?;
    let table_size = u32::try_from(table_size).map_err(|_| "function table exceeds32bits")?;
    if table_size > 64 * 1024 * 1024 || unwind_bytes.len() > 64 * 1024 * 1024 {
        return Err("unwind metadata exceeds image budget".into());
    }

    let mut out = spec.clone();
    let data_index = out.data.len();
    out.data.push(DataSection {
        name: ".xdata".into(),
        virtual_size: u32::try_from(unwind_bytes.len()).map_err(|_| "unwind size overflow")?,
        bytes: unwind_bytes,
        access: Access::ReadOnly,
        symbols: vec![],
    });
    out.data.push(DataSection {
        name: ".pdata".into(),
        virtual_size: table_size,
        bytes: vec![0; table_size as usize],
        access: Access::ReadOnly,
        symbols: vec![],
    });
    let layout = crate::pe::build(&out)?;
    let text_rva = layout.sections[0].rva;
    let unwind_rva = layout.sections[data_index + 1].rva;
    let mut records = vec![];
    for (f, offset) in functions.iter().zip(offsets) {
        records.push(RuntimeFunction {
            begin: text_rva
                .checked_add(f.begin_offset)
                .ok_or("function RVA overflow")?,
            end: text_rva
                .checked_add(f.end_offset)
                .ok_or("function RVA overflow")?,
            unwind: unwind_rva
                .checked_add(offset)
                .ok_or("unwind RVA overflow")?,
        });
    }
    out.data[data_index + 1].bytes = function_table(&records)?;
    out.exception_directory = Some(DirectoryRef {
        section: data_index + 1,
        offset: 0,
        size: table_size,
    });
    crate::pe::build(&out)?;
    Ok(out)
}
