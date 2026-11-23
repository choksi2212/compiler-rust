//! Native checked integer lowering. No interpreter, output replay or tool calls.
use crate::abi::{FramePlan, FrameRequest};
use crate::ir::{Ir, Value};
use crate::x64::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Success = 0,
    DivisionByZero = 1,
    Overflow = 2,
    Uninitialized = 3,
}
#[derive(Debug, Clone)]
pub struct NativeFunction {
    pub instructions: Vec<Instruction>,
    pub unwind: Vec<u8>,
    pub frame: FramePlan,
}
/// Explicit bindings to owned runtime code and readonly image data.
#[derive(Debug, Clone)]
pub struct RuntimeBindings {
    pub formatter: Label,
    pub constants: BTreeMap<usize, Symbol>,
    pub formats: BTreeMap<usize, Symbol>,
}
pub const RUNTIME_STATUS_OFFSET: i32 = 24;
struct Lower<'a> {
    string_slots: BTreeSet<usize>,
    code: Vec<Instruction>,
    homes: BTreeMap<usize, (u32, u32)>,
    labels: &'a mut LabelAllocator,
    divide: Label,
    overflow: Label,
    uninitialized: Label,
    exit: Label,
}
impl Lower<'_> {
    fn flag(&mut self, slot: usize, value: u64) {
        let offset = self.homes[&slot].1;
        let scratch = Reg::new(11).unwrap();
        self.code.push(Instruction::MovImm {
            width: Width::Dword,
            dst: scratch,
            value,
        });
        self.code.push(Instruction::StoreByte {
            address: Address::base(Reg::RSP, offset as i32),
            src: scratch,
        });
    }
    fn load(&mut self, slot: usize, dst: Reg) {
        let (offset, flag) = self.homes[&slot];
        let scratch = Reg::new(11).unwrap();
        self.code.extend([
            Instruction::LoadByte {
                dst: scratch,
                address: Address::base(Reg::RSP, flag as i32),
            },
            Instruction::Binary {
                width: Width::Dword,
                op: Binary::Test,
                dst: scratch,
                src: scratch,
            },
            Instruction::Branch {
                condition: Condition::Equal,
                label: self.uninitialized,
            },
            Instruction::Load {
                width: if self.string_slots.contains(&slot) {
                    Width::Qword
                } else {
                    Width::Dword
                },
                dst,
                address: Address::base(Reg::RSP, offset as i32),
            },
        ]);
    }
    fn store(&mut self, slot: usize) {
        self.code.push(Instruction::Store {
            width: if self.string_slots.contains(&slot) {
                Width::Qword
            } else {
                Width::Dword
            },
            address: Address::base(Reg::RSP, self.homes[&slot].0 as i32),
            src: Reg::RAX,
        });
        self.flag(slot, 1);
    }
    fn check_overflow(&mut self) {
        self.code.push(Instruction::Branch {
            condition: Condition::Overflow,
            label: self.overflow,
        });
    }
}
pub fn lower_integer(ir: &[Ir], labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    lower(ir, labels, None)
}
pub fn lower_with_runtime(
    ir: &[Ir],
    labels: &mut LabelAllocator,
    runtime: &RuntimeBindings,
) -> Result<NativeFunction, String> {
    lower(ir, labels, Some(runtime))
}
fn lower(
    ir: &[Ir],
    labels: &mut LabelAllocator,
    runtime: Option<&RuntimeBindings>,
) -> Result<NativeFunction, String> {
    crate::verify::verify(ir)?;
    let mut slots = BTreeSet::new();
    let mut targets = BTreeMap::new();
    let mut string_slots = BTreeSet::new();
    let mut max_arguments = 0;
    for (index, op) in ir.iter().enumerate() {
        match op {
            Ir::Const(d, Value::Str(_)) => {
                let bindings =
                    runtime.ok_or("integer native phase does not support string/print IR yet")?;
                if !bindings.constants.contains_key(&index) {
                    return Err("missing runtime literal binding".into());
                }
                slots.insert(*d);
                string_slots.insert(*d);
            }
            Ir::Print(_, arguments, d) => {
                let bindings =
                    runtime.ok_or("integer native phase does not support string/print IR yet")?;
                if !bindings.formats.contains_key(&index) {
                    return Err("missing runtime format binding".into());
                }
                max_arguments = max_arguments.max(arguments.len());
                slots.insert(*d);
            }
            Ir::Copy(d, a) => {
                slots.insert(*d);
                if string_slots.contains(a) {
                    string_slots.insert(*d);
                }
            }
            Ir::Unary(d, op, _) => {
                if op == "strlen" && runtime.is_none() {
                    return Err("integer native phase does not support strlen yet".into());
                }
                slots.insert(*d);
            }
            Ir::Reset(d) | Ir::Const(d, _) | Ir::Binary(d, ..) => {
                slots.insert(*d);
            }
            Ir::Label(l) => {
                targets.insert(*l, labels.fresh()?);
            }
            _ => (),
        }
    }
    if slots.len() > 65535 {
        return Err("integer native slot budget exceeded".into());
    }
    let argument_bytes = u32::try_from(
        max_arguments
            .checked_mul(8)
            .ok_or("print argument size overflow")?,
    )
    .map_err(|_| "print argument array too large")?;
    let frame = FramePlan::new(&FrameRequest {
        has_calls: runtime.is_some(),
        max_call_arguments: if runtime.is_some() { 4 } else { 0 },
        locals: if argument_bytes > 0 {
            vec![(argument_bytes, 8)]
        } else {
            vec![]
        },
        saved: if runtime.is_some() {
            vec![Reg::new(3)?]
        } else {
            vec![]
        },
        temporaries: slots.len(),
        initialization_flags: slots.len(),
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let homes = slots
        .iter()
        .enumerate()
        .map(|(index, id)| {
            (
                *id,
                (
                    frame.temporary_offsets()[index],
                    frame.flag_offsets()[index],
                ),
            )
        })
        .collect();
    let divide = labels.fresh()?;
    let overflow = labels.fresh()?;
    let uninitialized = labels.fresh()?;
    let exit = labels.fresh()?;
    let runtime_error = if runtime.is_some() {
        Some(labels.fresh()?)
    } else {
        None
    };
    let mut l = Lower {
        string_slots,
        code: prologue.instructions,
        homes,
        labels,
        divide,
        overflow,
        uninitialized,
        exit,
    };
    if runtime.is_some() {
        l.code.push(Instruction::Mov {
            width: Width::Qword,
            dst: Reg::new(3)?,
            src: Reg::RCX,
        });
    }
    let scratch = Reg::new(11)?;
    l.code.push(Instruction::MovImm {
        width: Width::Dword,
        dst: scratch,
        value: 0,
    });
    for id in &slots {
        l.code.push(Instruction::StoreByte {
            address: Address::base(Reg::RSP, l.homes[id].1 as i32),
            src: scratch,
        });
    }
    let right = Reg::new(10)?;
    for (instruction_index, op) in ir.iter().enumerate() {
        match op {
            Ir::Reset(d) => l.flag(*d, 0),
            Ir::Const(d, Value::Int(n)) => {
                l.code.push(Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: ((*n) as u32) as u64,
                });
                l.store(*d);
            }
            Ir::Const(d, Value::Str(_)) => {
                let symbol = runtime.ok_or("missing native runtime")?.constants[&instruction_index];
                l.code.push(Instruction::LeaSymbol {
                    dst: Reg::RAX,
                    symbol,
                });
                l.store(*d);
            }
            Ir::Print(_, arguments, d) => {
                let bindings = runtime.ok_or("missing native runtime")?;
                for (index, arg) in arguments.iter().enumerate() {
                    l.load(*arg, Reg::RAX);
                    l.code.push(Instruction::Store {
                        width: Width::Qword,
                        address: Address::base(
                            Reg::RSP,
                            frame.local_offsets()[0] as i32 + index as i32 * 8,
                        ),
                        src: Reg::RAX,
                    });
                }
                l.code.extend([
                    Instruction::Mov {
                        width: Width::Qword,
                        dst: Reg::RCX,
                        src: Reg::new(3)?,
                    },
                    Instruction::LeaSymbol {
                        dst: Reg::RDX,
                        symbol: bindings.formats[&instruction_index],
                    },
                ]);
                if arguments.is_empty() {
                    l.code.push(Instruction::MovImm {
                        width: Width::Qword,
                        dst: Reg::new(8)?,
                        value: 0,
                    });
                } else {
                    l.code.push(Instruction::Lea {
                        dst: Reg::new(8)?,
                        address: Address::base(Reg::RSP, frame.local_offsets()[0] as i32),
                    });
                }
                l.code.extend([
                    Instruction::MovImm {
                        width: Width::Dword,
                        dst: Reg::new(9)?,
                        value: arguments.len() as u64,
                    },
                    Instruction::Call(bindings.formatter),
                    Instruction::Binary {
                        width: Width::Dword,
                        op: Binary::Test,
                        dst: Reg::RAX,
                        src: Reg::RAX,
                    },
                    Instruction::Branch {
                        condition: Condition::Sign,
                        label: runtime_error.ok_or("missing runtime error label")?,
                    },
                ]);
                l.store(*d);
            }
            Ir::Unary(d, op, a) if op == "strlen" => {
                l.load(*a, right);
                let again = l.labels.fresh()?;
                let done = l.labels.fresh()?;
                l.code.extend([
                    Instruction::MovImm {
                        width: Width::Dword,
                        dst: Reg::RAX,
                        value: 0,
                    },
                    Instruction::Bind(again),
                    Instruction::LoadByte {
                        dst: scratch,
                        address: Address::base(right, 0),
                    },
                    Instruction::Binary {
                        width: Width::Dword,
                        op: Binary::Test,
                        dst: scratch,
                        src: scratch,
                    },
                    Instruction::Branch {
                        condition: Condition::Equal,
                        label: done,
                    },
                    Instruction::Immediate {
                        width: Width::Dword,
                        op: Immediate::Add,
                        reg: Reg::RAX,
                        value: 1,
                    },
                    Instruction::Immediate {
                        width: Width::Qword,
                        op: Immediate::Add,
                        reg: right,
                        value: 1,
                    },
                    Instruction::Jump(again),
                    Instruction::Bind(done),
                ]);
                l.store(*d);
            }
            Ir::Copy(d, a) => {
                l.load(*a, Reg::RAX);
                l.store(*d);
            }
            Ir::Unary(d, op, a) => {
                l.load(*a, Reg::RAX);
                if op == "-" {
                    l.code.push(Instruction::Negate {
                        width: Width::Dword,
                        reg: Reg::RAX,
                    });
                    l.check_overflow();
                } else if op != "+" {
                    l.code.extend([
                        Instruction::Binary {
                            width: Width::Dword,
                            op: Binary::Test,
                            dst: Reg::RAX,
                            src: Reg::RAX,
                        },
                        Instruction::Set {
                            condition: if op == "!" {
                                Condition::Equal
                            } else {
                                Condition::NotEqual
                            },
                            dst: Reg::RAX,
                        },
                    ]);
                }
                l.store(*d);
            }
            Ir::Binary(d, op, a, b) => {
                l.load(*a, Reg::RAX);
                l.load(*b, right);
                match op.as_str() {
                    "+" | "-" => {
                        l.code.push(Instruction::Binary {
                            width: Width::Dword,
                            op: if op == "+" { Binary::Add } else { Binary::Sub },
                            dst: Reg::RAX,
                            src: right,
                        });
                        l.check_overflow();
                    }
                    "*" => {
                        l.code.push(Instruction::Multiply {
                            width: Width::Dword,
                            dst: Reg::RAX,
                            src: right,
                        });
                        l.check_overflow();
                    }
                    "/" | "%" => {
                        let divide_ok = l.labels.fresh()?;
                        l.code.extend([
                            Instruction::Binary {
                                width: Width::Dword,
                                op: Binary::Test,
                                dst: right,
                                src: right,
                            },
                            Instruction::Branch {
                                condition: Condition::Equal,
                                label: l.divide,
                            },
                            Instruction::Immediate {
                                width: Width::Dword,
                                op: Immediate::Compare,
                                reg: Reg::RAX,
                                value: i32::MIN,
                            },
                            Instruction::Branch {
                                condition: Condition::NotEqual,
                                label: divide_ok,
                            },
                            Instruction::Immediate {
                                width: Width::Dword,
                                op: Immediate::Compare,
                                reg: right,
                                value: -1,
                            },
                            Instruction::Branch {
                                condition: Condition::Equal,
                                label: l.overflow,
                            },
                            Instruction::Bind(divide_ok),
                            Instruction::SignExtend(Width::Dword),
                            Instruction::Divide {
                                width: Width::Dword,
                                divisor: right,
                            },
                        ]);
                        if op == "%" {
                            l.code.push(Instruction::Mov {
                                width: Width::Dword,
                                dst: Reg::RAX,
                                src: Reg::RDX,
                            });
                        }
                    }
                    _ => {
                        let condition = match op.as_str() {
                            "<" => Condition::Less,
                            "<=" => Condition::LessEqual,
                            ">" => Condition::Greater,
                            ">=" => Condition::GreaterEqual,
                            "==" => Condition::Equal,
                            "!=" => Condition::NotEqual,
                            _ => return Err("unsupported native binary operator".into()),
                        };
                        l.code.extend([
                            Instruction::Binary {
                                width: Width::Dword,
                                op: Binary::Compare,
                                dst: Reg::RAX,
                                src: right,
                            },
                            Instruction::Set {
                                condition,
                                dst: Reg::RAX,
                            },
                        ]);
                    }
                }
                l.store(*d);
            }
            Ir::Label(label) => l.code.push(Instruction::Bind(targets[label])),
            Ir::Jump(label) => l.code.push(Instruction::Jump(targets[label])),
            Ir::Zero(a, label) => {
                l.load(*a, Reg::RAX);
                l.code.extend([
                    Instruction::Binary {
                        width: Width::Dword,
                        op: Binary::Test,
                        dst: Reg::RAX,
                        src: Reg::RAX,
                    },
                    Instruction::Branch {
                        condition: Condition::Equal,
                        label: targets[label],
                    },
                ]);
            }
            Ir::Return(a) => {
                l.load(*a, Reg::RAX);
                if runtime.is_some() {
                    l.code.push(Instruction::Store {
                        width: Width::Dword,
                        address: Address::base(Reg::new(3)?, 28),
                        src: Reg::RAX,
                    });
                    l.code.push(Instruction::MovImm {
                        width: Width::Dword,
                        dst: Reg::RAX,
                        value: 0,
                    });
                }
                l.code.push(Instruction::Jump(l.exit));
            }
            Ir::Halt => l.code.extend([
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: 0,
                },
                Instruction::Jump(l.exit),
            ]),
        }
    }
    for (label, status) in [
        (l.divide, Status::DivisionByZero),
        (l.overflow, Status::Overflow),
        (l.uninitialized, Status::Uninitialized),
    ] {
        l.code.extend([
            Instruction::Bind(label),
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: status as u64,
            },
            Instruction::Jump(l.exit),
        ]);
    }
    if let Some(error) = runtime_error {
        l.code.extend([
            Instruction::Bind(error),
            Instruction::Load {
                width: Width::Dword,
                dst: Reg::RAX,
                address: Address::base(Reg::new(3)?, RUNTIME_STATUS_OFFSET),
            },
            Instruction::Jump(l.exit),
        ]);
    }
    l.code.push(Instruction::Bind(l.exit));
    l.code.extend(frame.epilogue());
    if runtime.is_none() {
        encode(&l.code)?;
    }
    Ok(NativeFunction {
        instructions: l.code,
        unwind: prologue.unwind,
        frame,
    })
}
