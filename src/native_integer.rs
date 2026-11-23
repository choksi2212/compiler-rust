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
struct Lower<'a> {
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
                width: Width::Dword,
                dst,
                address: Address::base(Reg::RSP, offset as i32),
            },
        ]);
    }
    fn store(&mut self, slot: usize) {
        self.code.push(Instruction::Store {
            width: Width::Dword,
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
    crate::verify::verify(ir)?;
    let mut slots = BTreeSet::new();
    let mut targets = BTreeMap::new();
    for op in ir {
        match op {
            Ir::Const(_, Value::Str(_)) | Ir::Print(..) => {
                return Err("integer native phase does not support string/print IR yet".into());
            }
            Ir::Unary(_, op, _) if op == "strlen" => {
                return Err("integer native phase does not support strlen yet".into());
            }
            Ir::Reset(d)
            | Ir::Const(d, _)
            | Ir::Copy(d, _)
            | Ir::Unary(d, ..)
            | Ir::Binary(d, ..) => {
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
    let frame = FramePlan::new(&FrameRequest {
        temporaries: slots.len(),
        initialization_flags: slots.len(),
        ..FrameRequest::default()
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
    let mut l = Lower {
        code: prologue.instructions,
        homes,
        labels,
        divide,
        overflow,
        uninitialized,
        exit,
    };
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
    for op in ir {
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
                } else {
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
            Ir::Halt => l.code.extend([
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: 0,
                },
                Instruction::Jump(l.exit),
            ]),
            _ => return Err("unsupported native integer operation".into()),
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
    l.code.push(Instruction::Bind(l.exit));
    l.code.extend(frame.epilogue());
    encode(&l.code)?;
    Ok(NativeFunction {
        instructions: l.code,
        unwind: prologue.unwind,
        frame,
    })
}
