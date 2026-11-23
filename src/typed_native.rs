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
fn scalar_kind(types: &TypeContext, ty: TypeId) -> Result<IntegerKind, String> {
    match types.get(types.unqualified(ty)?)? {
        Type::Integer(kind) => Ok(*kind),
        Type::Pointer(_) => Ok(IntegerKind::UnsignedLongLong),
        _ => Err("scalar native type required".into()),
    }
}
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
#[derive(Debug)]
pub struct ModuleBindings {
    pub functions: HashMap<typed_ir::SymbolId, Label>,
    pub literals: HashMap<typed_ir::DataId, x64::Symbol>,
    pub context: x64::Symbol,
    pub globals: HashMap<typed_ir::SymbolId, (x64::Symbol, TypeId)>,
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
    bindings: Option<&'a ModuleBindings>,
    propagate: Label,
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
        let kind = scalar_kind(self.types, ty)?;
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
        let kind = scalar_kind(self.types, ty)?;
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
        let kind = scalar_kind(self.types, ty)?;
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
    fn global(&self, symbol: typed_ir::SymbolId) -> Result<(x64::Symbol, TypeId), String> {
        self.bindings
            .and_then(|b| b.globals.get(&symbol))
            .copied()
            .ok_or_else(|| format!("unresolved global symbol {}", symbol.0))
    }
    fn read_global(&mut self, dst: ValueId, symbol: typed_ir::SymbolId) -> Result<(), String> {
        self.load_global(symbol)?;
        self.write(dst, Reg::RAX);
        Ok(())
    }
    fn load_global(&mut self, symbol: typed_ir::SymbolId) -> Result<(), String> {
        let (symbol, ty) = self.global(symbol)?;
        let size = self.types.layout(ty)?.size;
        self.code.push(M::LeaSymbol {
            dst: reg(10),
            symbol,
        });
        let address = Address::base(reg(10), 0);
        self.code.push(match size {
            1 => M::LoadByte {
                dst: Reg::RAX,
                address,
            },
            2 => M::LoadWord {
                dst: Reg::RAX,
                address,
            },
            4 => M::Load {
                width: Width::Dword,
                dst: Reg::RAX,
                address,
            },
            8 => M::Load {
                width: W,
                dst: Reg::RAX,
                address,
            },
            _ => return Err("unsupported scalar global width".into()),
        });
        self.normalize(ty, Reg::RAX)?;
        Ok(())
    }
    fn write_global(&mut self, symbol: typed_ir::SymbolId, src: ValueId) -> Result<(), String> {
        self.read(src, Reg::RAX);
        self.store_global(symbol)
    }
    fn store_global(&mut self, symbol: typed_ir::SymbolId) -> Result<(), String> {
        let (symbol, ty) = self.global(symbol)?;
        let size = self.types.layout(ty)?.size;
        self.code.push(M::LeaSymbol {
            dst: reg(10),
            symbol,
        });
        let address = Address::base(reg(10), 0);
        self.code.push(match size {
            1 => M::StoreByte {
                address,
                src: Reg::RAX,
            },
            2 => M::StoreWord {
                address,
                src: Reg::RAX,
            },
            4 => M::Store {
                width: Width::Dword,
                address,
                src: Reg::RAX,
            },
            8 => M::Store {
                width: W,
                address,
                src: Reg::RAX,
            },
            _ => return Err("unsupported scalar global width".into()),
        });
        Ok(())
    }
    fn read_cursor(&mut self, variable: typed_ir::Variable) -> Result<(), String> {
        match variable {
            typed_ir::Variable::Local(local) => {
                self.code.extend([
                    M::LoadByte {
                        dst: reg(11),
                        address: Address::base(Reg::RSP, self.frame.flag_offsets()[local.0] as i32),
                    },
                    test(reg(11)),
                    branch(Condition::Equal, self.uninitialized),
                    load(Reg::RAX, self.frame.local_offsets()[local.0]),
                ]);
            }
            typed_ir::Variable::Global(symbol) => self.load_global(symbol)?,
        }
        self.code
            .extend([test(Reg::RAX), branch(Condition::Equal, self.uninitialized)]);
        Ok(())
    }
    fn write_cursor(&mut self, variable: typed_ir::Variable) -> Result<(), String> {
        match variable {
            typed_ir::Variable::Local(local) => {
                self.code
                    .push(store(self.frame.local_offsets()[local.0], Reg::RAX));
                self.flag(local, true);
            }
            typed_ir::Variable::Global(symbol) => self.store_global(symbol)?,
        }
        Ok(())
    }
    fn call(
        &mut self,
        dst: Option<ValueId>,
        arguments: &[ValueId],
        direct: Option<Label>,
        callee: Option<ValueId>,
    ) -> Result<(), String> {
        for (index, argument) in arguments.iter().enumerate() {
            self.read(*argument, Reg::RAX);
            match scalar_argument(index)? {
                ArgumentLocation::Register(target) => self.code.push(M::Mov {
                    width: W,
                    dst: target,
                    src: Reg::RAX,
                }),
                ArgumentLocation::Stack(offset) => self.code.push(store(offset, Reg::RAX)),
            }
        }
        if let Some(label) = direct {
            self.code.push(M::Call(label));
        } else {
            self.read(callee.ok_or("missing indirect callee")?, Reg::RAX);
            self.code.push(M::CallReg(Reg::RAX));
        }
        if self.bindings.is_some() || self.runtime {
            self.code.extend([
                M::Load {
                    width: Width::Dword,
                    dst: reg(11),
                    address: Address::base(reg(3), 24),
                },
                test(reg(11)),
                branch(Condition::NotEqual, self.propagate),
            ]);
        }
        if let Some(dst) = dst {
            self.normalize(self.value(dst), Reg::RAX)?;
            self.write(dst, Reg::RAX);
        }
        Ok(())
    }
    fn instruction(&mut self, i: &I) -> Result<(), String> {
        match i {
            I::VaStart { list, .. } => {
                let offset = self
                    .frame
                    .incoming_home_offset(self.function.parameters.len())?;
                self.code.push(M::Lea {
                    dst: Reg::RAX,
                    address: Address::base(Reg::RSP, offset as i32),
                });
                self.write_cursor(*list)?;
            }
            I::VaCopy {
                destination,
                source,
            } => {
                self.read_cursor(*source)?;
                self.write_cursor(*destination)?;
            }
            I::VaEnd { list } => {
                self.code.push(immediate(Reg::RAX, 0));
                self.write_cursor(*list)?;
                if let typed_ir::Variable::Local(local) = list {
                    self.flag(*local, false);
                }
            }
            I::VaArg { dst, list } => {
                self.read_cursor(*list)?;
                self.code.extend([
                    M::Mov {
                        width: W,
                        dst: reg(10),
                        src: Reg::RAX,
                    },
                    M::Load {
                        width: W,
                        dst: Reg::RAX,
                        address: Address::base(reg(10), 0),
                    },
                ]);
                self.normalize(self.value(*dst), Reg::RAX)?;
                self.write(*dst, Reg::RAX);
                self.code.extend([
                    M::Immediate {
                        width: W,
                        op: Immediate::Add,
                        reg: reg(10),
                        value: 8,
                    },
                    M::Mov {
                        width: W,
                        dst: Reg::RAX,
                        src: reg(10),
                    },
                ]);
                self.write_cursor(*list)?;
            }
            I::ReadGlobal { dst, symbol } => self.read_global(*dst, *symbol)?,
            I::WriteGlobal { symbol, src } => self.write_global(*symbol, *src)?,
            I::GlobalAddress { dst, symbol } => {
                let (symbol, _) = self.global(*symbol)?;
                self.code.push(M::LeaSymbol {
                    dst: Reg::RAX,
                    symbol,
                });
                self.write(*dst, Reg::RAX);
            }
            I::DirectCall {
                dst,
                symbol,
                arguments,
                ..
            } => {
                let label = self
                    .bindings
                    .and_then(|b| b.functions.get(symbol))
                    .copied()
                    .ok_or_else(|| format!("unresolved direct function symbol {}", symbol.0))?;
                self.call(*dst, arguments, Some(label), None)?;
            }
            I::Call {
                dst,
                callee,
                arguments,
                ..
            } => self.call(*dst, arguments, None, Some(*callee))?,
            I::DataAddress { dst, data } => {
                let symbol = self
                    .bindings
                    .and_then(|b| b.literals.get(data))
                    .copied()
                    .ok_or("literal linking requires module data bindings")?;
                self.code.push(M::LeaSymbol {
                    dst: Reg::RAX,
                    symbol,
                });
                self.write(*dst, Reg::RAX);
            }
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
                        if scalar_kind(self.types, self.value(*src))?.signed() {
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
            I::AddressLocal { .. } | I::Load { .. } | I::Store { .. } => {
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
            Terminator::Return(None) | Terminator::Fallthrough => {
                self.code.push(M::Jump(self.exit))
            }
            Terminator::Unreachable => self.code.push(M::Trap),
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
    build_body(module, &module.functions[0], labels, runtime, None)
}
pub fn lower_function(
    module: &Module,
    id: typed_ir::FunctionId,
    labels: &mut LabelAllocator,
    bindings: &ModuleBindings,
) -> Result<NativeFunction, String> {
    typed_ir::verify(module)?;
    let function = module
        .functions
        .iter()
        .find(|f| f.id == id)
        .ok_or("undefined function identity")?;
    build_body(module, function, labels, false, Some(bindings))
}
pub fn lower_functions(
    module: &Module,
    labels: &mut LabelAllocator,
    bindings: &ModuleBindings,
) -> Result<Vec<(typed_ir::FunctionId, NativeFunction)>, String> {
    typed_ir::verify(module)?;
    module
        .functions
        .iter()
        .map(|function| {
            build_body(module, function, labels, false, Some(bindings))
                .map(|body| (function.id, body))
        })
        .collect()
}
fn build_body(
    module: &Module,
    f: &Function,
    labels: &mut LabelAllocator,
    runtime: bool,
    bindings: Option<&ModuleBindings>,
) -> Result<NativeFunction, String> {
    let types = &module.types;
    let Type::Function {
        result, variadic, ..
    } = types.get(f.signature)?
    else {
        return Err("missing function type".into());
    };
    if *result != types.void() {
        scalar_kind(types, *result)?;
    }
    if runtime && (!f.parameters.is_empty() || *result != types.integer(IntegerKind::Int)) {
        return Err("runtime main requires int main(void)".into());
    }
    for ty in f
        .locals
        .iter()
        .map(|l| l.ty)
        .chain(f.values.iter().copied())
    {
        scalar_kind(types, ty)?;
    }
    let max_arguments = f
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter_map(|i| match i {
            I::Call { arguments, .. } | I::DirectCall { arguments, .. } => Some(arguments.len()),
            _ => None,
        })
        .max();
    let frame = FramePlan::new(&FrameRequest {
        has_calls: max_arguments.is_some(),
        max_call_arguments: max_arguments.unwrap_or(0),
        locals: vec![(8, 8); f.locals.len()],
        temporaries: f.values.len(),
        initialization_flags: f.locals.len(),
        saved: if runtime || bindings.is_some() {
            vec![reg(3)]
        } else {
            vec![]
        },
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut targets = HashMap::new();
    for block in &f.blocks {
        targets.insert(block.id, labels.fresh()?);
    }
    let propagate = labels.fresh()?;
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
        bindings,
        propagate,
    };
    if *variadic {
        for index in 0..4 {
            let ArgumentLocation::Register(source) = scalar_argument(index)? else {
                unreachable!()
            };
            l.code
                .push(store(l.frame.incoming_home_offset(index)?, source));
        }
    }
    if runtime {
        l.code.push(M::Mov {
            width: W,
            dst: reg(3),
            src: Reg::RCX,
        });
    }
    if let Some(bindings) = bindings {
        l.code.push(M::LeaSymbol {
            dst: reg(3),
            symbol: bindings.context,
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
        l.code.extend([M::Bind(label), immediate(Reg::RAX, status)]);
        if bindings.is_some() {
            l.code.extend([
                M::Store {
                    width: Width::Dword,
                    address: Address::base(reg(3), 24),
                    src: Reg::RAX,
                },
                immediate(Reg::RAX, 0),
            ]);
        }
        l.code.push(M::Jump(l.exit));
    }
    if bindings.is_some() || runtime {
        l.code.push(M::Bind(l.propagate));
        if runtime {
            l.code.push(M::Load {
                width: Width::Dword,
                dst: Reg::RAX,
                address: Address::base(reg(3), 24),
            });
        } else {
            l.code.push(immediate(Reg::RAX, 0));
        }
        l.code.push(M::Jump(l.exit));
    }
    l.code.push(M::Bind(l.exit));
    l.code.extend(l.frame.epilogue());
    if bindings.is_none() {
        x64::encode(&l.code)?;
    }
    Ok(NativeFunction {
        instructions: l.code,
        unwind: prologue.unwind,
        frame: l.frame,
    })
}
