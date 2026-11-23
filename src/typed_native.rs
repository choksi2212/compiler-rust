//! Native integer instruction selection for verified typed CFGs.
use crate::{
    abi::{ArgumentLocation, FramePlan, FrameRequest, scalar_argument},
    integer::{Binary as B, Unary},
    native_integer::NativeFunction,
    typed_ir::{self, Function, Instruction as I, LocalId, Module, Terminator, ValueId},
    types::{IntegerKind, Type, TypeContext, TypeId, mask},
    x64::{
        self, Address, Binary, Condition, Immediate, Instruction as M, Label, LabelAllocator, Reg,
        Shift, ShiftCount, Width,
    },
};
use std::collections::HashMap;
const W: Width = Width::Qword;
fn reg(n: u8) -> Reg {
    Reg::new(n).unwrap()
}
fn immediate(dst: Reg, value: u64) -> M {
    M::MovImm {
        width: W,
        dst,
        value,
    }
}
fn compare(a: Reg, b: Reg) -> M {
    M::Binary {
        width: W,
        op: Binary::Compare,
        dst: a,
        src: b,
    }
}
fn test(a: Reg) -> M {
    M::Binary {
        width: W,
        op: Binary::Test,
        dst: a,
        src: a,
    }
}
fn branch(condition: Condition, label: Label) -> M {
    M::Branch { condition, label }
}
fn load(dst: Reg, offset: u32) -> M {
    M::Load {
        width: W,
        dst,
        address: Address::base(Reg::RSP, offset as i32),
    }
}
fn store(offset: u32, src: Reg) -> M {
    M::Store {
        width: W,
        address: Address::base(Reg::RSP, offset as i32),
        src,
    }
}
struct Lower<'a> {
    types: &'a TypeContext,
    function: &'a Function,
    code: Vec<M>,
    frame: FramePlan,
    labels: &'a mut LabelAllocator,
    targets: HashMap<typed_ir::BlockId, Label>,
    overflow: Label,
    divide: Label,
    uninitialized: Label,
    bad_shift: Label,
    exit: Label,
    runtime: bool,
}
impl Lower<'_> {
    fn value(&self, id: ValueId) -> TypeId {
        self.function.values[id.0]
    }
    fn read(&mut self, id: ValueId, dst: Reg) {
        self.code
            .push(load(dst, self.frame.temporary_offsets()[id.0]));
    }
    fn write(&mut self, id: ValueId, src: Reg) {
        self.code
            .push(store(self.frame.temporary_offsets()[id.0], src));
    }
    fn flag(&mut self, id: LocalId, set: bool) {
        self.code.extend([
            immediate(reg(11), u64::from(set)),
            M::StoreByte {
                address: Address::base(Reg::RSP, self.frame.flag_offsets()[id.0] as i32),
                src: reg(11),
            },
        ]);
    }
    fn normalize(&mut self, ty: TypeId, r: Reg) -> Result<(), String> {
        let kind = self.types.integer_kind(ty)?;
        if kind == IntegerKind::Bool {
            self.code.extend([
                test(r),
                M::Set {
                    condition: Condition::NotEqual,
                    dst: r,
                },
            ]);
        } else if kind.bits() < 64 {
            if kind.signed() {
                let count = ShiftCount::Immediate((64 - kind.bits()) as u8);
                self.code.extend([
                    M::Shift {
                        width: W,
                        op: Shift::Left,
                        reg: r,
                        count,
                    },
                    M::Shift {
                        width: W,
                        op: Shift::ArithmeticRight,
                        reg: r,
                        count,
                    },
                ]);
            } else {
                self.code.extend([
                    immediate(reg(11), mask(kind.bits())),
                    M::Binary {
                        width: W,
                        op: Binary::And,
                        dst: r,
                        src: reg(11),
                    },
                ]);
            }
        }
        Ok(())
    }
    fn check_signed_range(&mut self, ty: TypeId) -> Result<(), String> {
        let kind = self.types.integer_kind(ty)?;
        if kind.signed() && kind.bits() < 64 {
            self.code.push(M::Mov {
                width: W,
                dst: reg(9),
                src: Reg::RAX,
            });
            self.normalize(ty, Reg::RAX)?;
            self.code.extend([
                compare(Reg::RAX, reg(9)),
                branch(Condition::NotEqual, self.overflow),
            ]);
        } else {
            self.normalize(ty, Reg::RAX)?;
        }
        Ok(())
    }
    fn operation(&mut self, dst: ValueId, op: B, a: ValueId, b: ValueId) -> Result<(), String> {
        let ty = self.value(a);
        let kind = self.types.integer_kind(ty)?;
        self.read(a, Reg::RAX);
        self.read(b, reg(10));
        match op {
            B::Add | B::Subtract | B::Multiply => {
                if op == B::Multiply {
                    self.code.push(M::Multiply {
                        width: W,
                        dst: Reg::RAX,
                        src: reg(10),
                    });
                } else {
                    self.code.push(M::Binary {
                        width: W,
                        op: if op == B::Add {
                            Binary::Add
                        } else {
                            Binary::Sub
                        },
                        dst: Reg::RAX,
                        src: reg(10),
                    });
                }
                if kind.signed() && kind.bits() == 64 {
                    self.code.push(branch(Condition::Overflow, self.overflow));
                }
                self.check_signed_range(ty)?;
            }
            B::And | B::Or | B::Xor => {
                let op = match op {
                    B::And => Binary::And,
                    B::Or => Binary::Or,
                    _ => Binary::Xor,
                };
                self.code.push(M::Binary {
                    width: W,
                    op,
                    dst: Reg::RAX,
                    src: reg(10),
                });
                self.normalize(ty, Reg::RAX)?;
            }
            B::Divide | B::Remainder => {
                self.code
                    .extend([test(reg(10)), branch(Condition::Equal, self.divide)]);
                if kind.signed() {
                    let normal = self.labels.fresh()?;
                    let minimum = if kind.bits() == 64 {
                        1u64 << 63
                    } else {
                        (-(1i64 << (kind.bits() - 1))) as u64
                    };
                    self.code.extend([
                        immediate(reg(11), minimum),
                        compare(Reg::RAX, reg(11)),
                        branch(Condition::NotEqual, normal),
                        M::Immediate {
                            width: W,
                            op: Immediate::Compare,
                            reg: reg(10),
                            value: -1,
                        },
                        branch(Condition::Equal, self.overflow),
                        M::Bind(normal),
                        M::SignExtend(W),
                        M::Divide {
                            width: W,
                            divisor: reg(10),
                        },
                    ]);
                } else {
                    self.code.extend([
                        immediate(Reg::RDX, 0),
                        M::DivideUnsigned {
                            width: W,
                            divisor: reg(10),
                        },
                    ]);
                }
                if op == B::Remainder {
                    self.code.push(M::Mov {
                        width: W,
                        dst: Reg::RAX,
                        src: Reg::RDX,
                    });
                }
                self.normalize(ty, Reg::RAX)?;
            }
            B::ShiftLeft | B::ShiftRight => {
                self.code.extend([
                    M::Immediate {
                        width: W,
                        op: Immediate::Compare,
                        reg: reg(10),
                        value: kind.bits() as i32,
                    },
                    branch(Condition::AboveEqual, self.bad_shift),
                    M::Mov {
                        width: W,
                        dst: Reg::RCX,
                        src: reg(10),
                    },
                ]);
                if op == B::ShiftLeft && kind.signed() {
                    self.code.extend([
                        test(Reg::RAX),
                        branch(Condition::Sign, self.bad_shift),
                        M::Mov {
                            width: W,
                            dst: reg(9),
                            src: Reg::RAX,
                        },
                    ]);
                }
                let shift = if op == B::ShiftLeft {
                    Shift::Left
                } else if kind.signed() {
                    Shift::ArithmeticRight
                } else {
                    Shift::LogicalRight
                };
                self.code.push(M::Shift {
                    width: W,
                    op: shift,
                    reg: Reg::RAX,
                    count: ShiftCount::Cl,
                });
                if op == B::ShiftLeft && kind.signed() {
                    self.code.extend([
                        M::Mov {
                            width: W,
                            dst: reg(11),
                            src: Reg::RAX,
                        },
                        M::Shift {
                            width: W,
                            op: Shift::ArithmeticRight,
                            reg: reg(11),
                            count: ShiftCount::Cl,
                        },
                        compare(reg(9), reg(11)),
                        branch(Condition::NotEqual, self.overflow),
                    ]);
                    self.check_signed_range(ty)?;
                } else {
                    self.normalize(ty, Reg::RAX)?;
                }
            }
            _ => {
                let condition = match (op, kind.signed()) {
                    (B::Equal, _) => Condition::Equal,
                    (B::NotEqual, _) => Condition::NotEqual,
                    (B::Less, true) => Condition::Less,
                    (B::Less, false) => Condition::Below,
                    (B::LessEqual, true) => Condition::LessEqual,
                    (B::LessEqual, false) => Condition::BelowEqual,
                    (B::Greater, true) => Condition::Greater,
                    (B::Greater, false) => Condition::Above,
                    (B::GreaterEqual, true) => Condition::GreaterEqual,
                    (B::GreaterEqual, false) => Condition::AboveEqual,
                    _ => return Err("unsupported typed integer binary operation".into()),
                };
                self.code.extend([
                    compare(Reg::RAX, reg(10)),
                    M::Set {
                        condition,
                        dst: Reg::RAX,
                    },
                ]);
            }
        }
        self.write(dst, Reg::RAX);
        Ok(())
    }
    fn instruction(&mut self, i: &I) -> Result<(), String> {
        match i {
            I::Constant { dst, bits } => {
                self.code.push(immediate(Reg::RAX, *bits));
                self.normalize(self.value(*dst), Reg::RAX)?;
                self.write(*dst, Reg::RAX);
            }
            I::Convert { dst, src } => {
                self.read(*src, Reg::RAX);
                self.normalize(self.value(*dst), Reg::RAX)?;
                self.write(*dst, Reg::RAX);
            }
            I::Binary { dst, op, lhs, rhs } => self.operation(*dst, *op, *lhs, *rhs)?,
            I::Unary { dst, op, src } => {
                self.read(*src, Reg::RAX);
                match op {
                    Unary::Plus => (),
                    Unary::Not => self.code.extend([
                        test(Reg::RAX),
                        M::Set {
                            condition: Condition::Equal,
                            dst: Reg::RAX,
                        },
                    ]),
                    Unary::Complement => self.code.extend([
                        immediate(reg(10), u64::MAX),
                        M::Binary {
                            width: W,
                            op: Binary::Xor,
                            dst: Reg::RAX,
                            src: reg(10),
                        },
                    ]),
                    Unary::Negate => {
                        self.code.push(M::Negate {
                            width: W,
                            reg: Reg::RAX,
                        });
                        if self.types.integer_kind(self.value(*src))?.signed() {
                            self.code.push(branch(Condition::Overflow, self.overflow));
                        }
                    }
                }
                if *op == Unary::Negate {
                    self.check_signed_range(self.value(*dst))?;
                } else {
                    self.normalize(self.value(*dst), Reg::RAX)?;
                }
                self.write(*dst, Reg::RAX);
            }
            I::ResetLocal(local) => self.flag(*local, false),
            I::InitializeLocal { local, src } | I::WriteLocal { local, src } => {
                self.read(*src, Reg::RAX);
                self.code
                    .push(store(self.frame.local_offsets()[local.0], Reg::RAX));
                self.flag(*local, true);
            }
            I::ReadLocal { dst, local } => {
                self.code.extend([
                    M::LoadByte {
                        dst: reg(11),
                        address: Address::base(Reg::RSP, self.frame.flag_offsets()[local.0] as i32),
                    },
                    test(reg(11)),
                    branch(Condition::Equal, self.uninitialized),
                    load(Reg::RAX, self.frame.local_offsets()[local.0]),
                ]);
                self.write(*dst, Reg::RAX);
            }
            I::AddressLocal { .. } | I::Load { .. } | I::Store { .. } | I::Call { .. } => {
                return Err(
                    "typed memory/call execution is not delivered by the integer type phase".into(),
                );
            }
        }
        Ok(())
    }
    fn terminator(&mut self, end: &Terminator) -> Result<(), String> {
        match end {
            Terminator::Jump(target) => self.code.push(M::Jump(self.targets[target])),
            Terminator::Branch { condition, yes, no } => {
                self.read(*condition, Reg::RAX);
                self.code.extend([
                    test(Reg::RAX),
                    branch(Condition::NotEqual, self.targets[yes]),
                    M::Jump(self.targets[no]),
                ]);
            }
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                self.read(*value, Reg::RAX);
                let ty = self.value(*value);
                for (bits, target) in cases {
                    self.code.push(immediate(reg(10), *bits));
                    self.normalize(ty, reg(10))?;
                    self.code.extend([
                        compare(Reg::RAX, reg(10)),
                        branch(Condition::Equal, self.targets[target]),
                    ]);
                }
                self.code.push(M::Jump(self.targets[default]));
            }
            Terminator::Return(Some(value)) => {
                self.read(*value, Reg::RAX);
                if self.runtime {
                    self.code.extend([
                        M::Store {
                            width: Width::Dword,
                            address: Address::base(reg(3), 28),
                            src: Reg::RAX,
                        },
                        immediate(Reg::RAX, 0),
                    ]);
                }
                self.code.push(M::Jump(self.exit));
            }
            Terminator::Return(None) => {
                return Err("integer native body requires an integer result".into());
            }
        }
        Ok(())
    }
}

/// Standalone ABI body, or runtime main body (RCX=context, no C parameters).
/// Runtime bodies return fault status separately from main's context result.
pub fn lower(
    module: &Module,
    labels: &mut LabelAllocator,
    runtime: bool,
) -> Result<NativeFunction, String> {
    typed_ir::verify(module)?;
    if !module.globals.is_empty() || module.functions.len() != 1 {
        return Err("typed integer phase requires one function and no global storage".into());
    }
    let f = &module.functions[0];
    let types = &module.types;
    let Type::Function { result, .. } = types.get(f.signature)? else {
        return Err("missing function type".into());
    };
    types.integer_kind(*result)?;
    if runtime && (!f.parameters.is_empty() || *result != types.integer(IntegerKind::Int)) {
        return Err("runtime main requires int main(void)".into());
    }
    for ty in f
        .locals
        .iter()
        .map(|l| l.ty)
        .chain(f.values.iter().copied())
    {
        types.integer_kind(ty)?;
    }
    let frame = FramePlan::new(&FrameRequest {
        locals: vec![(8, 8); f.locals.len()],
        temporaries: f.values.len(),
        initialization_flags: f.locals.len(),
        saved: if runtime { vec![reg(3)] } else { vec![] },
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut targets = HashMap::new();
    for block in &f.blocks {
        targets.insert(block.id, labels.fresh()?);
    }
    let mut l = Lower {
        types,
        function: f,
        code: prologue.instructions,
        frame,
        overflow: labels.fresh()?,
        divide: labels.fresh()?,
        uninitialized: labels.fresh()?,
        bad_shift: labels.fresh()?,
        exit: labels.fresh()?,
        labels,
        targets,
        runtime,
    };
    if runtime {
        l.code.push(M::Mov {
            width: W,
            dst: reg(3),
            src: Reg::RCX,
        });
    }
    for local in 0..f.locals.len() {
        l.flag(LocalId(local), false);
    }
    for (index, local) in f.parameters.iter().enumerate() {
        match scalar_argument(index)? {
            ArgumentLocation::Register(r) => l.code.push(M::Mov {
                width: W,
                dst: Reg::RAX,
                src: r,
            }),
            ArgumentLocation::Stack(_) => l
                .code
                .push(load(Reg::RAX, l.frame.incoming_stack_offset(index)?)),
        }
        l.normalize(f.locals[local.0].ty, Reg::RAX)?;
        l.code
            .push(store(l.frame.local_offsets()[local.0], Reg::RAX));
        l.flag(*local, true);
    }
    l.code.push(M::Jump(l.targets[&f.entry]));
    for block in &f.blocks {
        l.code.push(M::Bind(l.targets[&block.id]));
        for instruction in &block.instructions {
            l.instruction(instruction)?;
        }
        l.terminator(&block.end)?;
    }
    for (label, status) in [
        (l.divide, 1),
        (l.overflow, 2),
        (l.uninitialized, 3),
        (l.bad_shift, 8),
    ] {
        l.code
            .extend([M::Bind(label), immediate(Reg::RAX, status), M::Jump(l.exit)]);
    }
    l.code.push(M::Bind(l.exit));
    l.code.extend(l.frame.epilogue());
    x64::encode(&l.code)?;
    Ok(NativeFunction {
        instructions: l.code,
        unwind: prologue.unwind,
        frame: l.frame,
    })
}
