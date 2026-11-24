//! Typed non-SSA control-flow IR. Values have homes, locals have C lifetimes.
use crate::{
    integer::{Binary, Unary},
    types::{IntegerKind, IntegerValue, Type, TypeContext, TypeId},
};
use std::collections::{HashMap, HashSet, VecDeque};
macro_rules! identity {
    ($($name:ident),*) => {$(#[derive(Debug,Clone,Copy,PartialEq,Eq,Hash,PartialOrd,Ord)] pub struct $name(pub usize);)*};
}
identity!(FunctionId, BlockId, ValueId, LocalId, SymbolId, DataId);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variable {
    Local(LocalId),
    Global(SymbolId),
}
#[derive(Debug, Clone)]
pub struct Module {
    pub types: TypeContext,
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
    pub externs: Vec<External>,
    pub data: Vec<Data>,
    pub entry: Option<FunctionId>,
}
#[derive(Debug, Clone)]
pub struct External {
    pub linkage: crate::types::Linkage,
    pub symbol: SymbolId,
    pub name: String,
    pub signature: TypeId,
}
#[derive(Debug, Clone)]
pub struct Data {
    pub id: DataId,
    pub ty: TypeId,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct Global {
    pub alignment: Option<u32>,
    pub symbol: SymbolId,
    pub name: String,
    pub ty: TypeId,
    pub linkage: crate::types::Linkage,
    pub defined: bool,
    pub initial: GlobalInitializer,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InitializerStep {
    Element(u64),
    Member(usize),
}
#[derive(Debug, Clone)]
pub struct GlobalWrite {
    pub path: Vec<InitializerStep>,
    pub initial: GlobalInitializer,
}
#[derive(Debug, Clone)]
pub enum GlobalInitializer {
    Object(Vec<GlobalWrite>),
    Array(Vec<GlobalInitializer>),
    Zero,
    Integer(u64),
    Address { symbol: SymbolId, addend: i64 },
    DataAddress { data: DataId, addend: i64 },
}
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub linkage: crate::types::Linkage,
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
    pub alignment: Option<u32>,
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
    BitfieldRead {
        dst: ValueId,
        base: ValueId,
        path: Vec<usize>,
        volatile: bool,
    },
    BitfieldInitialize {
        dst: ValueId,
        base: ValueId,
        path: Vec<usize>,
        src: ValueId,
        volatile: bool,
    },
    ZeroObject {
        address: ValueId,
    },
    BitfieldWrite {
        dst: ValueId,
        base: ValueId,
        path: Vec<usize>,
        src: ValueId,
        volatile: bool,
    },
    AddressValue {
        dst: ValueId,
        src: ValueId,
    },
    MemberAddress {
        dst: ValueId,
        base: ValueId,
        path: Vec<usize>,
    },
    PointerOffset {
        dst: ValueId,
        base: ValueId,
        index: ValueId,
        subtract: bool,
    },
    PointerDifference {
        dst: ValueId,
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
    InitializeMemory {
        address: ValueId,
        src: ValueId,
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
    DirectCall {
        dst: Option<ValueId>,
        symbol: SymbolId,
        signature: TypeId,
        arguments: Vec<ValueId>,
    },
    DataAddress {
        dst: ValueId,
        data: DataId,
    },
    ReadGlobal {
        dst: ValueId,
        symbol: SymbolId,
    },
    WriteGlobal {
        symbol: SymbolId,
        src: ValueId,
    },
    FunctionAddress {
        dst: ValueId,
        symbol: SymbolId,
    },
    GlobalAddress {
        dst: ValueId,
        symbol: SymbolId,
    },
    VaStart {
        list: Variable,
        last: LocalId,
    },
    VaArg {
        dst: ValueId,
        list: Variable,
    },
    VaCopy {
        destination: Variable,
        source: Variable,
    },
    VaEnd {
        list: Variable,
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
    Unreachable,
    Fallthrough,
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
            | Self::BitfieldRead { dst, .. }
            | Self::BitfieldWrite { dst, .. }
            | Self::BitfieldInitialize { dst, .. }
            | Self::AddressValue { dst, .. }
            | Self::MemberAddress { dst, .. }
            | Self::PointerOffset { dst, .. }
            | Self::PointerDifference { dst, .. }
            | Self::ReadLocal { dst, .. }
            | Self::AddressLocal { dst, .. }
            | Self::Load { dst, .. } => Some(*dst),
            Self::Call { dst, .. } | Self::DirectCall { dst, .. } => *dst,
            Self::DataAddress { dst, .. }
            | Self::ReadGlobal { dst, .. }
            | Self::GlobalAddress { dst, .. }
            | Self::FunctionAddress { dst, .. }
            | Self::VaArg { dst, .. } => Some(*dst),
            Self::VaStart { .. }
            | Self::VaCopy { .. }
            | Self::VaEnd { .. }
            | Self::WriteGlobal { .. }
            | Self::WriteLocal { .. }
            | Self::InitializeLocal { .. }
            | Self::Store { .. }
            | Self::InitializeMemory { .. }
            | Self::ResetLocal(_)
            | Self::ZeroObject { .. } => None,
        }
    }
    pub fn operands(&self) -> Vec<ValueId> {
        match self {
            Self::Convert { src, .. }
            | Self::AddressValue { src, .. }
            | Self::Unary { src, .. }
            | Self::WriteLocal { src, .. }
            | Self::InitializeLocal { src, .. }
            | Self::WriteGlobal { src, .. } => {
                vec![*src]
            }
            Self::Binary { lhs, rhs, .. } | Self::PointerDifference { lhs, rhs, .. } => {
                vec![*lhs, *rhs]
            }
            Self::BitfieldRead { base, .. } => vec![*base],
            Self::BitfieldWrite { base, src, .. } | Self::BitfieldInitialize { base, src, .. } => {
                vec![*base, *src]
            }
            Self::ZeroObject { address } => vec![*address],
            Self::MemberAddress { base, .. } => vec![*base],
            Self::PointerOffset { base, index, .. } => vec![*base, *index],
            Self::Load { address, .. } => vec![*address],
            Self::Store { address, src, .. } | Self::InitializeMemory { address, src, .. } => {
                vec![*address, *src]
            }
            Self::DirectCall { arguments, .. } => arguments.clone(),
            Self::Call {
                callee, arguments, ..
            } => std::iter::once(*callee)
                .chain(arguments.iter().copied())
                .collect(),
            Self::Constant { .. }
            | Self::ReadLocal { .. }
            | Self::AddressLocal { .. }
            | Self::ResetLocal(_)
            | Self::DataAddress { .. }
            | Self::ReadGlobal { .. }
            | Self::GlobalAddress { .. }
            | Self::FunctionAddress { .. }
            | Self::VaStart { .. }
            | Self::VaArg { .. }
            | Self::VaCopy { .. }
            | Self::VaEnd { .. } => vec![],
        }
    }
    pub fn effects(&self, function: &Function, types: &TypeContext) -> Result<Effects, String> {
        Ok(match self {
            Self::BitfieldRead { volatile, .. } => Effects {
                read: true,
                observable: *volatile,
                may_trap: true,
                ..Default::default()
            },
            Self::ZeroObject { .. } => Effects {
                write: true,
                observable: true,
                may_trap: true,
                ..Default::default()
            },
            Self::BitfieldWrite { volatile, .. } | Self::BitfieldInitialize { volatile, .. } => {
                Effects {
                    read: true,
                    write: true,
                    observable: *volatile,
                    may_trap: true,
                }
            }
            Self::VaStart { .. }
            | Self::VaArg { .. }
            | Self::VaCopy { .. }
            | Self::VaEnd { .. } => Effects {
                read: true,
                write: true,
                observable: true,
                may_trap: true,
            },
            Self::ReadGlobal { .. } => Effects {
                read: true,
                observable: true,
                may_trap: true,
                ..Default::default()
            },
            Self::WriteGlobal { .. } => Effects {
                write: true,
                observable: true,
                ..Default::default()
            },
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
            Self::Store { volatile, .. } | Self::InitializeMemory { volatile, .. } => Effects {
                write: true,
                observable: *volatile,
                may_trap: true,
                ..Default::default()
            },
            Self::Call { .. } | Self::DirectCall { .. } => Effects {
                read: true,
                write: true,
                observable: true,
                may_trap: true,
            },
            Self::Unary { .. } | Self::Binary { .. } => Effects {
                may_trap: true,
                ..Default::default()
            },
            Self::Constant { .. }
            | Self::Convert { .. }
            | Self::AddressLocal { .. }
            | Self::DataAddress { .. }
            | Self::GlobalAddress { .. }
            | Self::FunctionAddress { .. }
            | Self::AddressValue { .. }
            | Self::MemberAddress { .. }
            | Self::PointerOffset { .. }
            | Self::PointerDifference { .. } => Effects::default(),
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
            Self::Return(_) | Self::Unreachable | Self::Fallthrough => vec![],
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
            Type::Integer(_) | Type::Enum(_) | Type::Pointer(_)
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
        !store || !types.contains_const(base)?,
        "store through const-qualified pointer",
    )
}

#[derive(Debug, Clone)]
pub struct InitializerLocation {
    pub ty: TypeId,
    pub offset: u64,
    pub bits: Option<crate::aggregate::BitField>,
}
pub fn initializer_path(
    types: &TypeContext,
    root: TypeId,
    path: &[InitializerStep],
) -> Result<InitializerLocation, String> {
    if path.len() > 128 {
        return Err("initializer path depth limit exceeded".into());
    }
    let mut ty = root;
    let mut offset = 0u64;
    let mut bits = None;
    for (at, step) in path.iter().enumerate() {
        match step {
            InitializerStep::Element(index) => {
                let Type::Array {
                    element,
                    length: Some(n),
                } = types.get(types.unqualified(ty)?)?
                else {
                    return Err("initializer element requires complete array".into());
                };
                if index >= n {
                    return Err("initializer element exceeds bound".into());
                }
                offset = offset
                    .checked_add(
                        index
                            .checked_mul(types.layout(*element)?.size)
                            .ok_or("initializer offset overflow")?,
                    )
                    .ok_or("initializer offset overflow")?;
                ty = *element;
            }
            InitializerStep::Member(index) => {
                let field = types
                    .record_type(ty)?
                    .fields
                    .as_ref()
                    .ok_or("initializer member of incomplete record")?
                    .get(*index)
                    .ok_or("invalid initializer member identity")?;
                if field.bits.is_some() && at + 1 != path.len() {
                    return Err("initializer descends within bitfield".into());
                }
                if field.bits.is_some_and(|b| b.width == 0) {
                    return Err("zero width field cannot be initialized".into());
                }
                offset = offset
                    .checked_add(field.offset)
                    .ok_or("initializer offset overflow")?;
                ty = field.ty;
                bits = field.bits;
            }
        }
    }
    types.layout(ty)?;
    Ok(InitializerLocation { ty, offset, bits })
}
fn verify_initial(
    module: &Module,
    storage: TypeId,
    initial: &GlobalInitializer,
) -> Result<(), String> {
    verify_initial_depth(module, storage, initial, 0)
}
fn verify_initial_depth(
    module: &Module,
    storage: TypeId,
    initial: &GlobalInitializer,
    depth: usize,
) -> Result<(), String> {
    if depth >= 128 {
        return Err("initializer nesting limit exceeded".into());
    }
    let types = &module.types;
    let ty = types.unqualified(storage)?;
    match initial {
        GlobalInitializer::Object(writes) => {
            require(
                matches!(types.get(ty)?, Type::Array { .. } | Type::Record(_)),
                "object initializer requires aggregate or array storage",
            )?;
            require(writes.len() <= 65535, "initializer write budget exceeded")?;
            let size = types.layout(ty)?.size;
            for write in writes {
                let location = initializer_path(types, ty, &write.path)?;
                require(
                    location
                        .offset
                        .checked_add(types.layout(location.ty)?.size)
                        .is_some_and(|end| end <= size),
                    "initializer write exceeds object",
                )?;
                if location.bits.is_some() {
                    require(
                        matches!(
                            write.initial,
                            GlobalInitializer::Integer(_) | GlobalInitializer::Zero
                        ),
                        "bitfield initializer must be integer constant",
                    )?;
                }
                verify_initial_depth(module, location.ty, &write.initial, depth + 1)?;
            }
        }
        GlobalInitializer::Array(values) => {
            let Type::Array {
                element,
                length: Some(n),
            } = types.get(ty)?
            else {
                return Err("array initializer requires complete array".into());
            };
            require(
                *n == values.len() as u64,
                "array initializer extent mismatch",
            )?;
            for value in values {
                verify_initial_depth(module, *element, value, depth + 1)?;
            }
        }
        GlobalInitializer::Zero => {
            types.layout(ty)?;
        }
        GlobalInitializer::Integer(bits) => IntegerValue { ty, bits: *bits }.validate(types)?,
        GlobalInitializer::Address { symbol, addend } => {
            pointer_target(types, ty)?;
            if module.functions.iter().any(|f| f.symbol == *symbol)
                || module.externs.iter().any(|f| f.symbol == *symbol)
            {
                require(
                    matches!(
                        types.get(pointer_target(types, ty)?)?,
                        Type::Function { .. }
                    ),
                    "function initializer requires function pointer",
                )?;
                require(*addend == 0, "function address cannot have an addend")?;
                return Ok(());
            }
            let target = module
                .globals
                .iter()
                .find(|g| g.symbol == *symbol)
                .ok_or("undefined initializer symbol")?;
            require(
                *addend >= 0 && (*addend as u64) <= types.layout(target.ty)?.size,
                "initializer address addend outside scalar object",
            )?;
        }
        GlobalInitializer::DataAddress { data, addend } => {
            pointer_target(types, ty)?;
            let data = module
                .data
                .iter()
                .find(|d| d.id == *data)
                .ok_or("undefined initializer data")?;
            require(
                *addend >= 0 && (*addend as u64) <= data.bytes.len() as u64,
                "initializer data addend outside object",
            )?;
        }
    }
    Ok(())
}
fn verify_alignment(types: &TypeContext, ty: TypeId, alignment: Option<u32>) -> Result<(), String> {
    if let Some(alignment) = alignment {
        fn natural(types: &TypeContext, ty: TypeId) -> Result<u32, String> {
            match types.get(types.unqualified(ty)?)? {
                Type::Array { element, .. } => natural(types, *element),
                _ => Ok(types.layout(ty)?.align),
            }
        }
        require(
            alignment.is_power_of_two() && alignment <= 16 && alignment >= natural(types, ty)?,
            "invalid object alignment metadata",
        )?;
    }
    Ok(())
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
        verify_alignment(types, global.ty, global.alignment)?;
        require(symbols.insert(global.symbol), "duplicate symbol identity")?;
        if global.defined {
            types.layout(global.ty)?;
        }
    }
    for external in &module.externs {
        require(
            symbols.insert(external.symbol),
            "duplicate external symbol identity",
        )?;
        require(
            matches!(types.get(external.signature)?, Type::Function { .. }),
            "external signature type required",
        )?;
    }
    let mut data_ids = HashSet::new();
    for data in &module.data {
        require(data_ids.insert(data.id), "duplicate data identity")?;
        require(
            types.layout(data.ty)?.size == data.bytes.len() as u64,
            "data layout/byte count mismatch",
        )?;
        require(
            matches!(types.get(data.ty)?, Type::Array { .. }),
            "literal data must be an array",
        )?;
    }
    if let Some(entry) = module.entry {
        require(
            module.functions.iter().any(|f| f.id == entry),
            "undefined module entry function",
        )?;
    }
    for global in &module.globals {
        if global.defined {
            verify_initial(module, global.ty, &global.initial)?;
        }
    }
    for function in &module.functions {
        require(functions.insert(function.id), "duplicate function identity")?;
        require(symbols.insert(function.symbol), "duplicate symbol identity")?;
        for local in &function.locals {
            verify_alignment(types, local.ty, local.alignment)?;
            require(symbols.insert(local.symbol), "duplicate symbol identity")?;
            types.layout(local.ty)?;
        }
        verify_function(module, function)
            .map_err(|e| format!("typed function {}: {e}", function.id.0))?;
    }
    Ok(())
}
fn verify_function(module: &Module, f: &Function) -> Result<(), String> {
    let types = &module.types;
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
        if types.aggregate_value(*ty)? {
            types.layout(*ty)?;
        } else {
            scalar(types, *ty)?;
        }
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
            check_instruction(module, f, instruction)
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
            Terminator::Fallthrough => require(
                types.unqualified(*result)? != types.void() && module.entry != Some(f.id),
                "invalid nonvoid fallthrough terminator",
            )?,
            Terminator::Jump(_) | Terminator::Unreachable => (),
        }
    }
    definite_values(f, &blocks)
}
fn check_instruction(module: &Module, f: &Function, i: &Instruction) -> Result<(), String> {
    let t = &module.types;
    let int = t.integer(IntegerKind::Int);
    match i {
        Instruction::Constant { dst, bits } => {
            let ty = f.value(*dst)?;
            if matches!(t.get(ty)?, Type::Pointer(_)) {
                require(
                    *bits == 0,
                    "non-null pointer constant requires a relocation/address operation",
                )
            } else {
                IntegerValue { ty, bits: *bits }.validate(t)
            }
        }
        Instruction::Convert { dst, src } => {
            if t.aggregate_value(f.value(*dst)?)? {
                return require(
                    f.value(*dst)? == f.value(*src)?,
                    "incompatible aggregate copy conversion",
                );
            }
            scalar(t, f.value(*src)?)?;
            scalar(t, f.value(*dst)?)
        }
        Instruction::Unary { dst, op, src } => {
            let source = f.value(*src)?;
            if *op == Unary::Not {
                scalar(t, source)?;
            } else {
                integer(t, source)?;
            }
            require(
                *op == Unary::Not || t.promoted(source)? == source,
                "unary operand lacks explicit promotion",
            )?;
            require(
                f.value(*dst)? == if *op == Unary::Not { int } else { source },
                "unary result type mismatch",
            )
        }
        Instruction::BitfieldRead {
            dst,
            base,
            path,
            volatile,
        }
        | Instruction::BitfieldWrite {
            dst,
            base,
            path,
            volatile,
            ..
        }
        | Instruction::BitfieldInitialize {
            dst,
            base,
            path,
            volatile,
            ..
        } => {
            let object = pointer_target(t, f.value(*base)?)?;
            let member = t.member_path(object, path)?;
            let bits = member
                .bits
                .ok_or("bitfield instruction names ordinary member")?;
            require(bits.width > 0, "zero-width bitfield is not a value")?;
            let ty = t.clone().member_type(object, path)?;
            let q = t.qualifiers(ty)?;
            require(
                f.value(*dst)? == t.unqualified(ty)? && (!q.volatile || *volatile),
                "bitfield value type or effect mismatch",
            )?;
            if let Instruction::BitfieldWrite { src, .. }
            | Instruction::BitfieldInitialize { src, .. } = i
            {
                require(
                    (!q.constant || matches!(i, Instruction::BitfieldInitialize { .. }))
                        && f.value(*src)? == f.value(*dst)?,
                    "invalid bitfield write type or constness",
                )?;
            }
            Ok(())
        }
        Instruction::ZeroObject { address } => {
            let ty = pointer_target(t, f.value(*address)?)?;
            t.layout(ty).map(|_| ())
        }
        Instruction::AddressValue { dst, src } => require(
            t.aggregate_value(f.value(*src)?)?
                && pointer_target(t, f.value(*dst)?)? == f.value(*src)?,
            "invalid aggregate value address",
        ),
        Instruction::MemberAddress { dst, base, path } => {
            let object = pointer_target(t, f.value(*base)?)?;
            let member = t.member_path(object, path)?;
            require(member.bits.is_none(), "cannot form a bitfield address")?;
            let field = t.clone().member_type(object, path)?;
            require(
                pointer_target(t, f.value(*dst)?)? == field,
                "member pointer type mismatch",
            )
        }
        Instruction::PointerOffset {
            dst, base, index, ..
        } => {
            let element = pointer_target(t, f.value(*base)?)?;
            t.layout(element)?;
            require(
                f.value(*dst)? == f.value(*base)?
                    && f.value(*index)? == t.integer(IntegerKind::LongLong),
                "invalid pointer offset types",
            )
        }
        Instruction::PointerDifference { dst, lhs, rhs } => {
            let a = pointer_target(t, f.value(*lhs)?)?;
            let b = pointer_target(t, f.value(*rhs)?)?;
            t.layout(a)?;
            require(
                t.compatible(t.unqualified(a)?, t.unqualified(b)?)?
                    && f.value(*dst)? == t.integer(IntegerKind::LongLong),
                "invalid pointer difference types",
            )
        }
        Instruction::Binary { dst, op, lhs, rhs } => {
            let (a, b) = (f.value(*lhs)?, f.value(*rhs)?);
            let pointers =
                matches!(t.get(a)?, Type::Pointer(_)) || matches!(t.get(b)?, Type::Pointer(_));
            if pointers {
                require(
                    op.comparison() && a == b,
                    "pointer binary operation requires compatible comparison operands",
                )?;
                if !matches!(
                    op,
                    crate::integer::Binary::Equal | crate::integer::Binary::NotEqual
                ) {
                    t.layout(pointer_target(t, a)?)?;
                }
            } else {
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
            }
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
                !t.contains_const(f.local(*local)?.ty)?,
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
        Instruction::InitializeMemory {
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
        Instruction::VaStart { list, last } => {
            let Type::Function { variadic, .. } = t.get(f.signature)? else {
                return Err("variadic operation lacks function type".into());
            };
            require(
                *variadic && f.parameters.last() == Some(last),
                "va_start names an invalid last parameter",
            )?;
            check_va_object(module, f, *list, true)
        }
        Instruction::VaArg { dst, list } => {
            check_va_object(module, f, *list, true)?;
            let ty = f.value(*dst)?;
            if t.aggregate_value(ty)? {
                t.layout(ty)?;
            } else {
                scalar(t, ty)?;
            }
            require(
                matches!(t.get(ty)?, Type::Pointer(_) | Type::Record(_)) || t.promoted(ty)? == ty,
                "va_arg result lacks default promotion",
            )
        }
        Instruction::VaCopy {
            destination,
            source,
        } => {
            check_va_object(module, f, *destination, true)?;
            check_va_object(module, f, *source, false)
        }
        Instruction::VaEnd { list } => check_va_object(module, f, *list, true),
        Instruction::ReadGlobal { dst, symbol } => {
            let global = module
                .globals
                .iter()
                .find(|g| g.symbol == *symbol)
                .ok_or("undefined global symbol")?;
            require(
                f.value(*dst)? == t.unqualified(global.ty)?,
                "global read type mismatch",
            )
        }
        Instruction::WriteGlobal { symbol, src } => {
            let global = module
                .globals
                .iter()
                .find(|g| g.symbol == *symbol)
                .ok_or("undefined global symbol")?;
            require(
                !t.contains_const(global.ty)?,
                "modification of const global",
            )?;
            require(
                f.value(*src)? == t.unqualified(global.ty)?,
                "global store requires explicit conversion",
            )
        }
        Instruction::FunctionAddress { dst, symbol } => {
            let signature = module
                .functions
                .iter()
                .find(|f| f.symbol == *symbol)
                .map(|f| f.signature)
                .or_else(|| {
                    module
                        .externs
                        .iter()
                        .find(|f| f.symbol == *symbol)
                        .map(|f| f.signature)
                })
                .ok_or("undefined function address symbol")?;
            require(
                pointer_target(t, f.value(*dst)?)? == signature,
                "function address type mismatch",
            )
        }
        Instruction::GlobalAddress { dst, symbol } => {
            let global = module
                .globals
                .iter()
                .find(|g| g.symbol == *symbol)
                .ok_or("undefined global address symbol")?;
            require(
                pointer_target(t, f.value(*dst)?)? == global.ty,
                "global address type mismatch",
            )
        }
        Instruction::DataAddress { dst, data } => {
            let record = module
                .data
                .iter()
                .find(|d| d.id == *data)
                .ok_or("undefined data identity")?;
            let Type::Array { element, .. } = t.get(record.ty)? else {
                return Err("literal data requires array type".into());
            };
            require(
                matches!(pointer_target(t,f.value(*dst)?)?,base if base==*element || base==record.ty),
                "literal address type mismatch",
            )
        }
        Instruction::DirectCall {
            dst,
            symbol,
            signature,
            arguments,
        } => {
            let resolved = module
                .functions
                .iter()
                .find(|f| f.symbol == *symbol)
                .map(|f| f.signature)
                .or_else(|| {
                    module
                        .externs
                        .iter()
                        .find(|f| f.symbol == *symbol)
                        .map(|f| f.signature)
                })
                .ok_or("undefined function symbol")?;
            require(
                t.compatible(resolved, *signature)?,
                "direct call signature mismatch",
            )?;
            check_arguments(t, f, *dst, *signature, arguments)
        }
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
            check_arguments(t, f, *dst, *signature, arguments)
        }
    }
}

fn check_va_object(
    module: &Module,
    f: &Function,
    variable: Variable,
    write: bool,
) -> Result<(), String> {
    let t = &module.types;
    let ty = match variable {
        Variable::Local(id) => f.local(id)?.ty,
        Variable::Global(id) => {
            module
                .globals
                .iter()
                .find(|g| g.symbol == id)
                .ok_or("undefined va_list global")?
                .ty
        }
    };
    require(
        !write || !t.qualifiers(ty)?.constant,
        "variadic cursor is const-qualified",
    )?;
    require(
        pointer_target(t, ty)? == t.integer(IntegerKind::Char),
        "variadic cursor requires the target va_list pointer representation",
    )
}
fn check_arguments(
    t: &TypeContext,
    f: &Function,
    destination: Option<ValueId>,
    signature_id: TypeId,
    arguments: &[ValueId],
) -> Result<(), String> {
    let signature = &signature_id;
    let dst = &destination;
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
        arguments.len() >= parameters.len() && (*variadic || arguments.len() == parameters.len()),
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
            matches!(t.get(ty)?, Type::Pointer(_) | Type::Record(_)) || t.promoted(ty)? == ty,
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
