//! Owned C ABI bridges to the native buffered runtime.
use crate::{
    abi::{FramePlan, FrameRequest},
    native_integer::NativeFunction,
    types::{IntegerKind, Linkage, TypeContext, TypeId},
    x64::{
        Address, Binary, Condition, Immediate, Instruction as M, Label, LabelAllocator, Reg,
        Symbol, Width,
    },
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Print,
    Strlen,
    Memcpy,
    Memmove,
    Memset,
    Strcmp,
    Strcpy,
    Malloc,
    Free,
    Calloc,
    Realloc,
}
pub fn service(
    types: &TypeContext,
    name: &str,
    signature: TypeId,
    linkage: Linkage,
) -> Result<Service, String> {
    use Service::*;
    let service = match name {
        "printf" => Print,
        "strlen" => Strlen,
        "memcpy" => Memcpy,
        "memmove" => Memmove,
        "memset" => Memset,
        "strcmp" => Strcmp,
        "strcpy" => Strcpy,
        "malloc" => Malloc,
        "free" => Free,
        "calloc" => Calloc,
        "realloc" => Realloc,
        _ => return Err(format!("unresolved external function {name}")),
    };
    if linkage != Linkage::External {
        return Err(format!("unresolved internal function {name}"));
    }
    let mut t = types.clone();
    let char_ = t.integer(IntegerKind::Char);
    let int = t.integer(IntegerKind::Int);
    let size = t.integer(IntegerKind::UnsignedLongLong);
    let character = t.pointer(char_)?;
    let constant = t.qualify(
        char_,
        crate::types::Qualifiers {
            constant: true,
            ..Default::default()
        },
    )?;
    let const_character = t.pointer(constant)?;
    let void = t.pointer(t.void())?;
    let constant = t.qualify(
        t.void(),
        crate::types::Qualifiers {
            constant: true,
            ..Default::default()
        },
    )?;
    let const_void = t.pointer(constant)?;
    let (result, parameters) = match service {
        Print => (int, vec![const_character]),
        Strlen => (size, vec![const_character]),
        Memcpy | Memmove => (void, vec![void, const_void, size]),
        Memset => (void, vec![void, int, size]),
        Strcmp => (int, vec![const_character, const_character]),
        Strcpy => (character, vec![character, const_character]),
        Malloc => (void, vec![size]),
        Free => (t.void(), vec![void]),
        Calloc => (void, vec![size, size]),
        Realloc => (void, vec![void, size]),
    };
    let expected = t.function(result, parameters, service == Print, true)?;
    if !t.compatible(signature, expected)? {
        return Err(format!("unsupported owned runtime signature for {name}"));
    }
    Ok(service)
}
pub fn bridge(
    service: Service,
    formatter: Label,
    context: Symbol,
    labels: &mut LabelAllocator,
) -> Result<NativeFunction, String> {
    if !matches!(service, Service::Print | Service::Strlen) {
        return crate::native_memory::bridge(service, labels);
    }
    let print = service == Service::Print;
    let frame = FramePlan::new(&FrameRequest {
        has_calls: print,
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let cursor = Reg::new(10)?;
    let byte = Reg::new(11)?;
    let count = Reg::new(9)?;
    let again = labels.fresh()?;
    let done = labels.fresh()?;
    if print {
        for (i, reg) in [Reg::RCX, Reg::RDX, Reg::new(8)?, count]
            .into_iter()
            .enumerate()
        {
            code.push(M::Store {
                width: Width::Qword,
                address: Address::base(Reg::RSP, frame.incoming_home_offset(i)? as i32),
                src: reg,
            });
        }
    }
    code.extend([
        M::Mov {
            width: Width::Qword,
            dst: cursor,
            src: Reg::RCX,
        },
        M::MovImm {
            width: Width::Qword,
            dst: if print { count } else { Reg::RAX },
            value: 0,
        },
        M::Bind(again),
        M::LoadByte {
            dst: byte,
            address: Address::base(cursor, 0),
        },
        M::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: byte,
            src: byte,
        },
        M::Branch {
            condition: Condition::Equal,
            label: done,
        },
        M::Immediate {
            width: Width::Qword,
            op: Immediate::Add,
            reg: cursor,
            value: 1,
        },
    ]);
    if print {
        code.extend([
            M::Immediate {
                width: Width::Dword,
                op: Immediate::Compare,
                reg: byte,
                value: 37,
            },
            M::Branch {
                condition: Condition::NotEqual,
                label: again,
            },
            M::Immediate {
                width: Width::Qword,
                op: Immediate::Add,
                reg: count,
                value: 1,
            },
        ]);
    } else {
        code.push(M::Immediate {
            width: Width::Qword,
            op: Immediate::Add,
            reg: Reg::RAX,
            value: 1,
        });
    }
    code.extend([M::Jump(again), M::Bind(done)]);
    if print {
        code.extend([
            M::LeaSymbol {
                dst: Reg::RCX,
                symbol: context,
            },
            M::Load {
                width: Width::Qword,
                dst: Reg::RDX,
                address: Address::base(Reg::RSP, frame.incoming_home_offset(0)? as i32),
            },
            M::Lea {
                dst: Reg::new(8)?,
                address: Address::base(Reg::RSP, frame.incoming_home_offset(1)? as i32),
            },
            M::Call(formatter),
        ]);
    }
    code.extend(frame.epilogue());
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
