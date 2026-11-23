//! Typed non-SSA control-flow IR. Values have homes, locals have C lifetimes.
use crate::{
    integer::{Binary, Unary},
    types::{IntegerKind, IntegerValue, Type, TypeContext, TypeId},
};
use std::collections::{HashMap, HashSet, VecDeque};
macro_rules! identity {
    ($($name:ident),*) => {$(#[derive(Debug,Clone,Copy,PartialEq,Eq,Hash,PartialOrd,Ord)] pub struct $name(pub usize);)*};
}
identity!(FunctionId, BlockId, ValueId, LocalId, SymbolId);
#[derive(Debug, Clone)]
pub struct Module {
    pub types: TypeContext,
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
}
#[derive(Debug, Clone)]
pub struct Global {
    pub symbol: SymbolId,
    pub ty: TypeId,
}
#[derive(Debug, Clone)]
pub struct Function {
    pub id: FunctionId,
    pub symbol: SymbolId,
    pub signature: TypeId,
    pub parameters: Vec<LocalId>,
    pub locals: Vec<Local>,
    pub values: Vec<TypeId>,
    pub entry: BlockId,
    pub blocks: Vec<Block>,
}
#[derive(Debug, Clone)]
pub struct Local {
    pub symbol: SymbolId,
    pub ty: TypeId,
}
#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    pub instructions: Vec<Instruction>,
    pub end: Terminator,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instruction {
    Constant {
        dst: ValueId,
        bits: u64,
    },
    Convert {
        dst: ValueId,
        src: ValueId,
    },
    Unary {
        dst: ValueId,
        op: Unary,
        src: ValueId,
    },
    Binary {
        dst: ValueId,
        op: Binary,
        lhs: ValueId,
        rhs: ValueId,
    },
    ReadLocal {
        dst: ValueId,
        local: LocalId,
    },
    WriteLocal {
        local: LocalId,
        src: ValueId,
    },
    InitializeLocal {
        local: LocalId,
        src: ValueId,
    },
    ResetLocal(LocalId),
    AddressLocal {
        dst: ValueId,
        local: LocalId,
    },
    Load {
        dst: ValueId,
        address: ValueId,
        align: u32,
        volatile: bool,
    },
    Store {
        address: ValueId,
        src: ValueId,
        align: u32,
        volatile: bool,
    },
    Call {
        dst: Option<ValueId>,
        callee: ValueId,
        signature: TypeId,
        arguments: Vec<ValueId>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Terminator {
    Jump(BlockId),
    Branch {
        condition: ValueId,
        yes: BlockId,
        no: BlockId,
    },
    Switch {
        value: ValueId,
        cases: Vec<(u64, BlockId)>,
        default: BlockId,
    },
    Return(Option<ValueId>),
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Effects {
    pub read: bool,
    pub write: bool,
    pub observable: bool,
    pub may_trap: bool,
}
impl Instruction {
    pub fn destination(&self) -> Option<ValueId> {
        match self {
            Self::Constant { dst, .. }
            | Self::Convert { dst, .. }
            | Self::Unary { dst, .. }
            | Self::Binary { dst, .. }
            | Self::ReadLocal { dst, .. }
            | Self::AddressLocal { dst, .. }
            | Self::Load { dst, .. } => Some(*dst),
            Self::Call { dst, .. } => *dst,
            Self::WriteLocal { .. }
            | Self::InitializeLocal { .. }
            | Self::Store { .. }
            | Self::ResetLocal(_) => None,
        }
    }
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Convert { src, .. }
            | Self::Unary { src, .. }
            | Self::WriteLocal { src, .. }
            | Self::InitializeLocal { src, .. } => {
                vec![*src]
            }
            Self::Binary { lhs, rhs, .. } => vec![*lhs, *rhs],
            Self::Load { address, .. } => vec![*address],
            Self::Store { address, src, .. } => vec![*address, *src],
            Self::Call {
                callee, arguments, ..
            } => std::iter::once(*callee)
                .chain(arguments.iter().copied())
                .collect(),
            Self::Constant { .. }
            | Self::ReadLocal { .. }
            | Self::AddressLocal { .. }
            | Self::ResetLocal(_) => vec![],
        }
    }
    pub fn effects(&self, function: &Function, types: &TypeContext) -> Result<Effects, String> {
        Ok(match self {
            Self::ReadLocal { local, .. } => Effects {
                read: true,
                observable: types.qualifiers(function.local(*local)?.ty)?.volatile,
                may_trap: true,
                ..Default::default()
            },
            Self::WriteLocal { local, .. } | Self::InitializeLocal { local, .. } => Effects {
                write: true,
                observable: types.qualifiers(function.local(*local)?.ty)?.volatile,
                ..Default::default()
            },
            Self::ResetLocal(_) => Effects {
                write: true,
                ..Default::default()
            },
            Self::Load { volatile, .. } => Effects {
                read: true,
                observable: *volatile,
                may_trap: true,
                ..Default::default()
            },
            Self::Store { volatile, .. } => Effects {
                write: true,
                observable: *volatile,
                may_trap: true,
                ..Default::default()
            },
            Self::Call { .. } => Effects {
                read: true,
                write: true,
                observable: true,
                may_trap: true,
            },
            Self::Unary { .. } | Self::Binary { .. } => Effects {
                may_trap: true,
                ..Default::default()
            },
            Self::Constant { .. } | Self::Convert { .. } | Self::AddressLocal { .. } => {
                Effects::default()
            }
        })
    }
}
impl Terminator {
    pub fn targets(&self) -> Vec<BlockId> {
        match self {
            Self::Jump(b) => vec![*b],
            Self::Branch { yes, no, .. } => vec![*yes, *no],
            Self::Switch { cases, default, .. } => std::iter::once(*default)
                .chain(cases.iter().map(|c| c.1))
                .collect(),
            Self::Return(_) => vec![],
        }
    }
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Branch { condition, .. } => vec![*condition],
            Self::Switch { value, .. } => vec![*value],
            Self::Return(Some(value)) => vec![*value],
            _ => vec![],
        }
    }
}
impl Function {
    pub fn value(&self, id: ValueId) -> Result<TypeId, String> {
        self.values
            .get(id.0)
            .copied()
            .ok_or_else(|| format!("undefined typed value {}", id.0))
    }
    pub fn local(&self, id: LocalId) -> Result<&Local, String> {
        self.locals
            .get(id.0)
            .ok_or_else(|| format!("undefined local {}", id.0))
    }
}
fn require(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn scalar(types: &TypeContext, ty: TypeId) -> Result<(), String> {
    require(
        matches!(
            types.get(types.unqualified(ty)?)?,
            Type::Integer(_) | Type::Pointer(_)
        ),
        "scalar type required",
    )
}
fn integer(types: &TypeContext, ty: TypeId) -> Result<(), String> {
    types.integer_kind(ty).map(|_| ())
}
fn pointer_target(types: &TypeContext, ty: TypeId) -> Result<TypeId, String> {
    match types.get(types.unqualified(ty)?)? {
        Type::Pointer(base) => Ok(*base),
        _ => Err("pointer operand required".into()),
    }
}
fn memory(
    types: &TypeContext,
    pointer: TypeId,
    value: TypeId,
    align: u32,
    volatile: bool,
    store: bool,
) -> Result<(), String> {
    let base = pointer_target(types, pointer)?;
    require(
        types.unqualified(base)? == value,
        "memory value does not match pointed-to type",
    )?;
    types.layout(base)?;
    require(
        align.is_power_of_two() && align <= 1 << 20,
        "invalid memory alignment",
    )?;
    let qualifiers = types.qualifiers(base)?;
    require(
        !qualifiers.volatile || volatile,
        "volatile access lost its effect",
    )?;
    require(
        !store || !qualifiers.constant,
        "store through const-qualified pointer",
    )
}

pub fn verify(module: &Module) -> Result<(), String> {
    let types = &module.types;
    let mut symbols = HashSet::new();
    let mut functions = HashSet::new();
    require(
        module.functions.len() <= 4096 && module.globals.len() <= 65535,
        "module identity budget exceeded",
    )?;
    for global in &module.globals {
        require(symbols.insert(global.symbol), "duplicate symbol identity")?;
        types.layout(global.ty)?;
    }
    for function in &module.functions {
        require(functions.insert(function.id), "duplicate function identity")?;
        require(symbols.insert(function.symbol), "duplicate symbol identity")?;
        for local in &function.locals {
            require(symbols.insert(local.symbol), "duplicate symbol identity")?;
            types.layout(local.ty)?;
        }
        verify_function(types, function)
            .map_err(|e| format!("typed function {}: {e}", function.id.0))?;
    }
    Ok(())
}
fn verify_function(types: &TypeContext, f: &Function) -> Result<(), String> {
    require(
        !f.blocks.is_empty()
            && f.blocks.len() <= 4096
            && f.values.len() <= 65535
            && f.locals.len() <= 65535,
        "function IR budget exceeded",
    )?;
    let Type::Function {
        result,
        parameters,
        prototype,
        ..
    } = types.get(f.signature)?
    else {
        return Err("function signature type required".into());
    };
    require(*prototype, "defined function requires a complete prototype")?;
    require(
        parameters.len() == f.parameters.len(),
        "parameter local count mismatch",
    )?;
    let mut parameter_ids = HashSet::new();
    for (parameter, ty) in f.parameters.iter().zip(parameters) {
        require(
            parameter_ids.insert(*parameter),
            "duplicate parameter local",
        )?;
        require(
            types.unqualified(f.local(*parameter)?.ty)? == *ty,
            "parameter local type mismatch",
        )?;
    }
    for ty in &f.values {
        scalar(types, *ty)?;
        require(
            types.unqualified(*ty)? == *ty,
            "value type retains an lvalue qualifier",
        )?;
    }
    let mut blocks = HashMap::new();
    for (i, block) in f.blocks.iter().enumerate() {
        require(
            blocks.insert(block.id, i).is_none(),
            "duplicate block identity",
        )?;
    }
    require(blocks.contains_key(&f.entry), "missing entry block")?;
    for block in &f.blocks {
        for target in block.end.targets() {
            require(blocks.contains_key(&target), "undefined branch target")?;
        }
        for instruction in &block.instructions {
            check_instruction(types, f, instruction)
                .map_err(|e| format!("block {}: {e}", block.id.0))?;
        }
        match &block.end {
            Terminator::Branch { condition, .. } => scalar(types, f.value(*condition)?)?,
            Terminator::Switch { value, cases, .. } => {
                let ty = f.value(*value)?;
                integer(types, ty)?;
                require(
                    types.promoted(ty)? == ty,
                    "switch requires a promoted controlling type",
                )?;
                let mut distinct = HashSet::new();
                for (bits, _) in cases {
                    IntegerValue { ty, bits: *bits }.validate(types)?;
                    require(distinct.insert(*bits), "duplicate converted switch case")?;
                }
            }
            Terminator::Return(value) => match value {
                Some(value) => require(
                    f.value(*value)? == types.unqualified(*result)?,
                    "return value requires explicit conversion",
                )?,
                None => require(
                    *result == types.void(),
                    "nonvoid function return has no value",
                )?,
            },
            Terminator::Jump(_) => (),
        }
    }
    definite_values(f, &blocks)
}
fn check_instruction(t: &TypeContext, f: &Function, i: &Instruction) -> Result<(), String> {
    let int = t.integer(IntegerKind::Int);
    match i {
        Instruction::Constant { dst, bits } => IntegerValue {
            ty: f.value(*dst)?,
            bits: *bits,
        }
        .validate(t),
        Instruction::Convert { dst, src } => {
            integer(t, f.value(*src)?)?;
            integer(t, f.value(*dst)?)
        }
        Instruction::Unary { dst, op, src } => {
            let source = f.value(*src)?;
            integer(t, source)?;
            require(
                *op == Unary::Not || t.promoted(source)? == source,
                "unary operand lacks explicit promotion",
            )?;
            require(
                f.value(*dst)? == if *op == Unary::Not { int } else { source },
                "unary result type mismatch",
            )
        }
        Instruction::Binary { dst, op, lhs, rhs } => {
            let (a, b) = (f.value(*lhs)?, f.value(*rhs)?);
            integer(t, a)?;
            integer(t, b)?;
            require(
                t.promoted(a)? == a && t.promoted(b)? == b,
                "binary operand lacks explicit promotion",
            )?;
            require(
                op.shift() || a == b,
                "binary operands lack common conversion",
            )?;
            require(
                f.value(*dst)? == if op.comparison() { int } else { a },
                "binary result type mismatch",
            )
        }
        Instruction::ReadLocal { dst, local } => require(
            f.value(*dst)? == t.unqualified(f.local(*local)?.ty)?,
            "local read type mismatch",
        ),
        Instruction::WriteLocal { local, src } => {
            require(
                !t.qualifiers(f.local(*local)?.ty)?.constant,
                "modification of a const-qualified local",
            )?;
            require(
                t.unqualified(f.local(*local)?.ty)? == f.value(*src)?,
                "local store requires explicit conversion",
            )
        }
        Instruction::InitializeLocal { local, src } => require(
            t.unqualified(f.local(*local)?.ty)? == f.value(*src)?,
            "local initializer requires explicit conversion",
        ),
        Instruction::ResetLocal(local) => {
            f.local(*local)?;
            Ok(())
        }
        Instruction::AddressLocal { dst, local } => require(
            pointer_target(t, f.value(*dst)?)? == f.local(*local)?.ty,
            "local address type mismatch",
        ),
        Instruction::Load {
            dst,
            address,
            align,
            volatile,
        } => memory(
            t,
            f.value(*address)?,
            f.value(*dst)?,
            *align,
            *volatile,
            false,
        ),
        Instruction::Store {
            address,
            src,
            align,
            volatile,
        } => memory(
            t,
            f.value(*address)?,
            f.value(*src)?,
            *align,
            *volatile,
            true,
        ),
        Instruction::Call {
            dst,
            callee,
            signature,
            arguments,
        } => {
            require(
                pointer_target(t, f.value(*callee)?)? == *signature,
                "callee signature mismatch",
            )?;
            let Type::Function {
                result,
                parameters,
                variadic,
                prototype,
            } = t.get(*signature)?
            else {
                return Err("call signature required".into());
            };
            require(*prototype, "call requires a resolved promoted prototype")?;
            require(
                arguments.len() >= parameters.len()
                    && (*variadic || arguments.len() == parameters.len()),
                "call argument count mismatch",
            )?;
            for (argument, parameter) in arguments.iter().zip(parameters) {
                require(
                    f.value(*argument)? == *parameter,
                    "call argument requires explicit conversion",
                )?;
            }
            for argument in &arguments[parameters.len()..] {
                let ty = f.value(*argument)?;
                require(
                    matches!(t.get(ty)?, Type::Pointer(_)) || t.promoted(ty)? == ty,
                    "variadic argument lacks default promotion",
                )?;
            }
            match dst {
                Some(dst) => require(
                    f.value(*dst)? == t.unqualified(*result)?,
                    "call result type mismatch",
                ),
                None => require(
                    *result == t.void(),
                    "nonvoid call must record its returned value",
                ),
            }
        }
    }
}

// Non-SSA must-definition analysis: every temporary read must be defined on all
// reachable predecessor paths. Local indeterminacy is a separate runtime check.
fn definite_values(f: &Function, blocks: &HashMap<BlockId, usize>) -> Result<(), String> {
    let n = f.blocks.len();
    let words = f.values.len().div_ceil(64);
    require(
        n.checked_mul(words).is_some_and(|cells| cells <= 4_194_304),
        "value-flow analysis memory budget exceeded",
    )?;
    let mut reachable = vec![false; n];
    let mut work = vec![blocks[&f.entry]];
    while let Some(i) = work.pop() {
        if std::mem::replace(&mut reachable[i], true) {
            continue;
        }
        for target in f.blocks[i].end.targets() {
            work.push(blocks[&target]);
        }
    }
    let mut predecessors = vec![vec![]; n];
    for (i, b) in f.blocks.iter().enumerate() {
        if reachable[i] {
            for target in b.end.targets() {
                predecessors[blocks[&target]].push(i);
            }
        }
    }
    let mut incoming = vec![vec![u64::MAX; words]; n];
    let mut outgoing = incoming.clone();
    let mut queue: VecDeque<usize> = (0..n).filter(|i| reachable[*i]).collect();
    let mut visits = 0usize;
    while let Some(i) = queue.pop_front() {
        visits += 1;
        require(
            visits <= 1_000_000,
            "value-flow analysis work budget exceeded",
        )?;
        let mut available = vec![u64::MAX; words];
        if f.blocks[i].id == f.entry {
            available.fill(0);
        } else {
            for p in &predecessors[i] {
                for (a, b) in available.iter_mut().zip(&outgoing[*p]) {
                    *a &= b;
                }
            }
        }
        incoming[i] = available.clone();
        for instruction in &f.blocks[i].instructions {
            if let Some(dst) = instruction.destination() {
                available[dst.0 / 64] |= 1u64 << (dst.0 % 64);
            }
        }
        if outgoing[i] != available {
            outgoing[i] = available;
            for target in f.blocks[i].end.targets() {
                queue.push_back(blocks[&target]);
            }
        }
    }
    for (i, b) in f.blocks.iter().enumerate() {
        if !reachable[i] {
            continue;
        }
        let mut available = incoming[i].clone();
        let check = |operand: ValueId, available: &[u64]| {
            require(
                available[operand.0 / 64] & (1u64 << (operand.0 % 64)) != 0,
                "temporary used before definition on a reachable path",
            )
        };
        for instruction in &b.instructions {
            for operand in instruction.operands() {
                check(operand, &available)?;
            }
            if let Some(dst) = instruction.destination() {
                available[dst.0 / 64] |= 1u64 << (dst.0 % 64);
            }
        }
        for operand in b.end.operands() {
            check(operand, &available)?;
        }
    }
    Ok(())
}
