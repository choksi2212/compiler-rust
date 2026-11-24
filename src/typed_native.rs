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
        Type::Enum(_) => Ok(IntegerKind::Int),
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
    pub function_addresses: HashMap<typed_ir::SymbolId, x64::Symbol>,
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
    addressed: std::collections::HashSet<LocalId>,
    hidden_return: Option<u32>,
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
    fn temporary_address(&mut self, id: ValueId, dst: Reg) {
        self.code.push(M::Lea {
            dst,
            address: Address::base(Reg::RSP, self.frame.temporary_offsets()[id.0] as i32),
        });
    }
    fn local_address(&mut self, id: LocalId, dst: Reg) {
        self.code.push(M::Lea {
            dst,
            address: Address::base(Reg::RSP, self.frame.local_offsets()[id.0] as i32),
        });
    }
    /// R10=destination, R11=source; copy exactly the target object extent.
    fn copy_object(&mut self, ty: TypeId) -> Result<(), String> {
        let bytes = self.types.layout(ty)?.size;
        if bytes == 0 {
            return Err("zero-size aggregate copy".into());
        }
        let again = self.labels.fresh()?;
        self.code.extend([
            immediate(reg(8), bytes),
            M::Bind(again),
            M::LoadByte {
                dst: Reg::RAX,
                address: Address::base(reg(11), 0),
            },
            M::StoreByte {
                address: Address::base(reg(10), 0),
                src: Reg::RAX,
            },
            M::Immediate {
                width: W,
                op: Immediate::Add,
                reg: reg(10),
                value: 1,
            },
            M::Immediate {
                width: W,
                op: Immediate::Add,
                reg: reg(11),
                value: 1,
            },
            M::Immediate {
                width: W,
                op: Immediate::Sub,
                reg: reg(8),
                value: 1,
            },
            test(reg(8)),
            branch(Condition::NotEqual, again),
        ]);
        Ok(())
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
        self.code.push(M::LeaSymbol {
            dst: reg(10),
            symbol,
        });
        self.load_object(ty)
    }
    fn load_object(&mut self, ty: TypeId) -> Result<(), String> {
        let size = self.types.layout(ty)?.size;
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
        if !self.types.aggregate_value(ty)? {
            self.normalize(ty, Reg::RAX)?;
        }
        Ok(())
    }
    fn write_global(&mut self, symbol: typed_ir::SymbolId, src: ValueId) -> Result<(), String> {
        self.read(src, Reg::RAX);
        self.store_global(symbol)
    }
    fn store_global(&mut self, symbol: typed_ir::SymbolId) -> Result<(), String> {
        let (symbol, ty) = self.global(symbol)?;
        self.code.push(M::LeaSymbol {
            dst: reg(10),
            symbol,
        });
        self.store_object(ty)
    }
    fn store_object(&mut self, ty: TypeId) -> Result<(), String> {
        let size = self.types.layout(ty)?.size;
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
        let hidden = match dst {
            Some(dst) => self.types.indirect_aggregate(self.value(dst))?,
            None => false,
        };
        if hidden {
            self.temporary_address(dst.ok_or("hidden return destination missing")?, Reg::RCX);
        }
        for (index, argument) in arguments.iter().enumerate() {
            if self.types.aggregate_value(self.value(*argument))? {
                if self.types.indirect_aggregate(self.value(*argument))? {
                    self.temporary_address(*argument, Reg::RAX);
                } else {
                    self.temporary_address(*argument, reg(10));
                    self.load_object(self.value(*argument))?;
                }
            } else {
                self.read(*argument, Reg::RAX);
            }
            match scalar_argument(index + usize::from(hidden))? {
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
            if self.types.aggregate_value(self.value(dst))? {
                if !hidden {
                    self.temporary_address(dst, reg(10));
                    self.store_object(self.value(dst))?;
                }
            } else {
                self.normalize(self.value(dst), Reg::RAX)?;
                self.write(dst, Reg::RAX);
            }
        }
        Ok(())
    }
    fn bitfield_normalize(&mut self, register: Reg, width: u32, signed: bool) {
        if width < 64 {
            if signed {
                let count = ShiftCount::Immediate((64 - width) as u8);
                self.code.extend([
                    M::Shift {
                        width: W,
                        op: Shift::Left,
                        reg: register,
                        count,
                    },
                    M::Shift {
                        width: W,
                        op: Shift::ArithmeticRight,
                        reg: register,
                        count,
                    },
                ]);
            } else {
                self.code.extend([
                    immediate(reg(11), mask(width)),
                    M::Binary {
                        width: W,
                        op: Binary::And,
                        dst: register,
                        src: reg(11),
                    },
                ]);
            }
        }
    }
    fn bitfield(
        &mut self,
        dst: ValueId,
        base: ValueId,
        path: &[usize],
        source: Option<ValueId>,
    ) -> Result<(), String> {
        let Type::Pointer(object) = self.types.get(self.value(base))? else {
            unreachable!()
        };
        let member = self.types.member_path(*object, path)?;
        let bits = member.bits.ok_or("bitfield metadata missing")?;
        let signed = self.types.integer_kind(bits.storage)?.signed();
        self.read(base, reg(10));
        self.code.extend([
            immediate(reg(11), member.offset),
            M::Binary {
                width: W,
                op: Binary::Add,
                dst: reg(10),
                src: reg(11),
            },
        ]);
        if let Some(source) = source {
            self.read(source, reg(8));
            self.bitfield_normalize(reg(8), bits.width, false);
            self.code.push(M::Mov {
                width: W,
                dst: reg(9),
                src: reg(8),
            });
            self.bitfield_normalize(reg(9), bits.width, signed);
            self.load_object(bits.storage)?;
            self.code.extend([
                immediate(reg(11), !(mask(bits.width) << bits.shift)),
                M::Binary {
                    width: W,
                    op: Binary::And,
                    dst: Reg::RAX,
                    src: reg(11),
                },
            ]);
            if bits.shift != 0 {
                self.code.push(M::Shift {
                    width: W,
                    op: Shift::Left,
                    reg: reg(8),
                    count: ShiftCount::Immediate(bits.shift as u8),
                });
            }
            self.code.push(M::Binary {
                width: W,
                op: Binary::Or,
                dst: Reg::RAX,
                src: reg(8),
            });
            self.store_object(bits.storage)?;
            self.write(dst, reg(9));
        } else {
            self.load_object(bits.storage)?;
            if bits.shift != 0 {
                self.code.push(M::Shift {
                    width: W,
                    op: Shift::LogicalRight,
                    reg: Reg::RAX,
                    count: ShiftCount::Immediate(bits.shift as u8),
                });
            }
            self.bitfield_normalize(Reg::RAX, bits.width, signed);
            self.write(dst, Reg::RAX);
        }
        Ok(())
    }
    fn instruction(&mut self, i: &I) -> Result<(), String> {
        match i {
            I::ZeroObject { address } => {
                let Type::Pointer(ty) = self.types.get(self.value(*address))? else {
                    return Err("zero initializer requires object pointer".into());
                };
                let bytes = self.types.layout(*ty)?.size;
                if bytes != 0 {
                    let again = self.labels.fresh()?;
                    self.read(*address, reg(10));
                    self.code.extend([
                        immediate(reg(8), bytes),
                        immediate(Reg::RAX, 0),
                        M::Bind(again),
                        M::StoreByte {
                            address: Address::base(reg(10), 0),
                            src: Reg::RAX,
                        },
                        M::Immediate {
                            width: W,
                            op: Immediate::Add,
                            reg: reg(10),
                            value: 1,
                        },
                        M::Immediate {
                            width: W,
                            op: Immediate::Sub,
                            reg: reg(8),
                            value: 1,
                        },
                        test(reg(8)),
                        branch(Condition::NotEqual, again),
                    ]);
                }
            }
            I::BitfieldRead {
                dst, base, path, ..
            } => self.bitfield(*dst, *base, path, None)?,
            I::BitfieldWrite {
                dst,
                base,
                path,
                src,
                ..
            }
            | I::BitfieldInitialize {
                dst,
                base,
                path,
                src,
                ..
            } => self.bitfield(*dst, *base, path, Some(*src))?,
            I::VaStart { list, .. } => {
                let offset = self.frame.incoming_home_offset(
                    self.function.parameters.len() + usize::from(self.hidden_return.is_some()),
                )?;
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
                if self.types.aggregate_value(self.value(*dst))? {
                    self.read_cursor(*list)?;
                    self.code.push(M::Load {
                        width: W,
                        dst: Reg::RAX,
                        address: Address::base(Reg::RAX, 0),
                    });
                    if self.types.indirect_aggregate(self.value(*dst))? {
                        self.code.push(M::Mov {
                            width: W,
                            dst: reg(11),
                            src: Reg::RAX,
                        });
                        self.temporary_address(*dst, reg(10));
                        self.copy_object(self.value(*dst))?;
                    } else {
                        self.temporary_address(*dst, reg(10));
                        self.store_object(self.value(*dst))?;
                    }
                    self.read_cursor(*list)?;
                    self.code.push(M::Immediate {
                        width: W,
                        op: Immediate::Add,
                        reg: Reg::RAX,
                        value: 8,
                    });
                    self.write_cursor(*list)?;
                    return Ok(());
                }
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
            I::ReadGlobal { dst, symbol } => {
                if self.types.aggregate_value(self.value(*dst))? {
                    let (symbol, ty) = self.global(*symbol)?;
                    self.code.push(M::LeaSymbol {
                        dst: reg(11),
                        symbol,
                    });
                    self.temporary_address(*dst, reg(10));
                    self.copy_object(ty)?;
                } else {
                    self.read_global(*dst, *symbol)?;
                }
            }
            I::WriteGlobal { symbol, src } => {
                if self.types.aggregate_value(self.value(*src))? {
                    let (symbol, ty) = self.global(*symbol)?;
                    self.code.push(M::LeaSymbol {
                        dst: reg(10),
                        symbol,
                    });
                    self.temporary_address(*src, reg(11));
                    self.copy_object(ty)?;
                } else {
                    self.write_global(*symbol, *src)?;
                }
            }
            I::FunctionAddress { dst, symbol } => {
                let symbol = self
                    .bindings
                    .and_then(|b| b.function_addresses.get(symbol))
                    .copied()
                    .ok_or("unresolved function address")?;
                self.code.push(M::LeaSymbol {
                    dst: Reg::RAX,
                    symbol,
                });
                self.write(*dst, Reg::RAX);
            }
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
            I::AddressValue { dst, src } => {
                self.temporary_address(*src, Reg::RAX);
                self.write(*dst, Reg::RAX);
            }
            I::Convert { dst, src } => {
                if self.types.aggregate_value(self.value(*dst))? {
                    self.temporary_address(*src, reg(11));
                    self.temporary_address(*dst, reg(10));
                    self.copy_object(self.value(*dst))?;
                    return Ok(());
                }
                self.read(*src, Reg::RAX);
                self.normalize(self.value(*dst), Reg::RAX)?;
                self.write(*dst, Reg::RAX);
            }
            I::MemberAddress { dst, base, path } => {
                let Type::Pointer(object) = self.types.get(self.value(*base))? else {
                    unreachable!()
                };
                let offset = self.types.member_path(*object, path)?.offset;
                self.read(*base, Reg::RAX);
                self.code.extend([
                    immediate(reg(10), offset),
                    M::Binary {
                        width: W,
                        op: Binary::Add,
                        dst: Reg::RAX,
                        src: reg(10),
                    },
                ]);
                self.write(*dst, Reg::RAX);
            }
            I::PointerOffset {
                dst,
                base,
                index,
                subtract,
            } => {
                let Type::Pointer(element) = self.types.get(self.value(*base))? else {
                    unreachable!()
                };
                let stride = self.types.layout(*element)?.size;
                self.read(*base, Reg::RAX);
                self.read(*index, reg(10));
                self.code.extend([
                    immediate(reg(11), stride),
                    M::Multiply {
                        width: W,
                        dst: reg(10),
                        src: reg(11),
                    },
                    M::Binary {
                        width: W,
                        op: if *subtract { Binary::Sub } else { Binary::Add },
                        dst: Reg::RAX,
                        src: reg(10),
                    },
                ]);
                self.write(*dst, Reg::RAX);
            }
            I::PointerDifference { dst, lhs, rhs } => {
                let Type::Pointer(element) = self.types.get(self.value(*lhs))? else {
                    unreachable!()
                };
                let stride = self.types.layout(*element)?.size;
                self.read(*lhs, Reg::RAX);
                self.read(*rhs, reg(10));
                self.code.extend([
                    M::Binary {
                        width: W,
                        op: Binary::Sub,
                        dst: Reg::RAX,
                        src: reg(10),
                    },
                    immediate(reg(10), stride),
                    M::SignExtend(W),
                    M::Divide {
                        width: W,
                        divisor: reg(10),
                    },
                ]);
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
                if self.types.aggregate_value(self.value(*src))? {
                    self.temporary_address(*src, reg(11));
                    self.local_address(*local, reg(10));
                    self.copy_object(self.value(*src))?;
                    self.flag(*local, true);
                    return Ok(());
                }
                self.read(*src, Reg::RAX);
                self.code
                    .push(store(self.frame.local_offsets()[local.0], Reg::RAX));
                self.flag(*local, true);
            }
            I::ReadLocal { dst, local } => {
                if !self.addressed.contains(local) {
                    self.code.extend([
                        M::LoadByte {
                            dst: reg(11),
                            address: Address::base(
                                Reg::RSP,
                                self.frame.flag_offsets()[local.0] as i32,
                            ),
                        },
                        test(reg(11)),
                        branch(Condition::Equal, self.uninitialized),
                    ]);
                }
                if self.types.aggregate_value(self.value(*dst))? {
                    self.local_address(*local, reg(11));
                    self.temporary_address(*dst, reg(10));
                    self.copy_object(self.value(*dst))?;
                    return Ok(());
                }
                self.code.push(M::Lea {
                    dst: reg(10),
                    address: Address::base(Reg::RSP, self.frame.local_offsets()[local.0] as i32),
                });
                self.load_object(self.value(*dst))?;
                self.write(*dst, Reg::RAX);
            }
            I::AddressLocal { dst, local } => {
                self.code.push(M::Lea {
                    dst: Reg::RAX,
                    address: Address::base(Reg::RSP, self.frame.local_offsets()[local.0] as i32),
                });
                self.write(*dst, Reg::RAX);
            }
            I::Load { dst, address, .. } => {
                if self.types.aggregate_value(self.value(*dst))? {
                    self.read(*address, reg(11));
                    self.temporary_address(*dst, reg(10));
                    self.copy_object(self.value(*dst))?;
                    return Ok(());
                }
                self.read(*address, reg(10));
                self.load_object(self.value(*dst))?;
                self.write(*dst, Reg::RAX);
            }
            I::Store { address, src, .. } | I::InitializeMemory { address, src, .. } => {
                if self.types.aggregate_value(self.value(*src))? {
                    self.read(*address, reg(10));
                    self.temporary_address(*src, reg(11));
                    self.copy_object(self.value(*src))?;
                    return Ok(());
                }
                self.read(*address, reg(10));
                self.read(*src, Reg::RAX);
                self.store_object(self.value(*src))?;
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
                if self.types.aggregate_value(self.value(*value))? {
                    if let Some(home) = self.hidden_return {
                        self.code.push(load(reg(10), home));
                        self.temporary_address(*value, reg(11));
                        self.copy_object(self.value(*value))?;
                        self.code.push(load(Reg::RAX, home));
                    } else {
                        self.temporary_address(*value, reg(10));
                        self.load_object(self.value(*value))?;
                    }
                    self.code.push(M::Jump(self.exit));
                    return Ok(());
                }
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
        if types.aggregate_value(*result)? {
            types.layout(*result)?;
        } else {
            scalar_kind(types, *result)?;
        }
    }
    let hidden = types.indirect_aggregate(*result)?;
    if runtime && (!f.parameters.is_empty() || *result != types.integer(IntegerKind::Int)) {
        return Err("runtime main requires int main(void)".into());
    }
    for local in &f.locals {
        types.layout(local.ty)?;
    }
    for ty in &f.values {
        if types.aggregate_value(*ty)? {
            types.layout(*ty)?;
        } else {
            scalar_kind(types, *ty)?;
        }
    }
    let mut max_arguments = None;
    for instruction in f.blocks.iter().flat_map(|b| &b.instructions) {
        if let I::Call { dst, arguments, .. } | I::DirectCall { dst, arguments, .. } = instruction {
            let shift = if let Some(dst) = dst {
                usize::from(types.indirect_aggregate(f.values[dst.0])?)
            } else {
                0
            };
            let n = arguments.len() + shift;
            max_arguments = Some(max_arguments.unwrap_or(0).max(n));
        }
    }
    let mut local_layouts = f
        .locals
        .iter()
        .map(|local| {
            let layout = types.layout(local.ty)?;
            Ok((
                u32::try_from(layout.size.max(8))
                    .map_err(|_| "local object exceeds frame budget")?,
                local.alignment.unwrap_or(layout.align).max(8),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if hidden {
        local_layouts.push((8, 8));
    }
    let frame = FramePlan::new(&FrameRequest {
        has_calls: max_arguments.is_some(),
        max_call_arguments: max_arguments.unwrap_or(0),
        locals: local_layouts,
        temporaries: f.values.len(),
        temporary_layouts: f
            .values
            .iter()
            .map(|ty| {
                if types.aggregate_value(*ty)? {
                    let layout = types.layout(*ty)?;
                    Ok((
                        u32::try_from(layout.size.max(8))
                            .map_err(|_| "aggregate temporary exceeds frame limit")?,
                        16,
                    ))
                } else {
                    Ok((8, 8))
                }
            })
            .collect::<Result<Vec<_>, String>>()?,
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
        hidden_return: if hidden {
            Some(frame.local_offsets()[f.locals.len()])
        } else {
            None
        },
        frame,
        overflow: labels.fresh()?,
        divide: labels.fresh()?,
        uninitialized: labels.fresh()?,
        addressed: f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| {
                if let I::AddressLocal { local, .. } = i {
                    Some(*local)
                } else {
                    None
                }
            })
            .collect(),
        bad_shift: labels.fresh()?,
        exit: labels.fresh()?,
        labels,
        targets,
        runtime,
        bindings,
        propagate,
    };
    let mut home_arguments = *variadic || hidden;
    for local in &f.parameters {
        home_arguments |= types.aggregate_value(f.locals[local.0].ty)?;
    }
    if home_arguments {
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
    if let Some(home) = l.hidden_return {
        l.code
            .push(load(Reg::RAX, l.frame.incoming_home_offset(0)?));
        l.code.push(store(home, Reg::RAX));
    }
    for local in 0..f.locals.len() {
        l.flag(LocalId(local), false);
    }
    for (index, local) in f.parameters.iter().enumerate() {
        let index = index + usize::from(hidden);
        if home_arguments {
            l.code
                .push(load(Reg::RAX, l.frame.incoming_home_offset(index)?));
        } else {
            match scalar_argument(index)? {
                ArgumentLocation::Register(src) => l.code.push(M::Mov {
                    width: W,
                    dst: Reg::RAX,
                    src,
                }),
                ArgumentLocation::Stack(_) => l
                    .code
                    .push(load(Reg::RAX, l.frame.incoming_stack_offset(index)?)),
            }
        }
        let ty = f.locals[local.0].ty;
        if types.aggregate_value(ty)? {
            if types.indirect_aggregate(ty)? {
                l.code.push(M::Mov {
                    width: W,
                    dst: reg(11),
                    src: Reg::RAX,
                });
                l.local_address(*local, reg(10));
                l.copy_object(ty)?;
            } else {
                l.local_address(*local, reg(10));
                l.store_object(ty)?;
            }
        } else {
            l.normalize(ty, Reg::RAX)?;
            l.code
                .push(store(l.frame.local_offsets()[local.0], Reg::RAX));
        }
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
