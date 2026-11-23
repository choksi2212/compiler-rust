//! Owned C ABI bridges to the native buffered runtime.
use crate::{
    abi::{FramePlan, FrameRequest},
    native_integer::NativeFunction,
    types::{IntegerKind, Linkage, Type, TypeContext, TypeId},
    x64::{
        Address, Binary, Condition, Immediate, Instruction as M, Label, LabelAllocator, Reg,
        Symbol, Width,
    },
};
pub fn service(
    types: &TypeContext,
    name: &str,
    signature: TypeId,
    linkage: Linkage,
) -> Result<bool, String> {
    if linkage != Linkage::External || !matches!(name, "printf" | "strlen") {
        return Err(format!("unresolved external function {name}"));
    }
    let Type::Function {
        result,
        parameters,
        variadic,
        prototype,
    } = types.get(signature)?
    else {
        return Err("runtime function signature required".into());
    };
    let character_pointer = if parameters.len() == 1 {
        if let Type::Pointer(base) = types.get(types.unqualified(parameters[0])?)? {
            types.unqualified(*base)? == types.integer(IntegerKind::Char)
                && types.qualifiers(*base)?.constant
        } else {
            false
        }
    } else {
        false
    };
    let print = name == "printf";
    let expected = types.integer(if print {
        IntegerKind::Int
    } else {
        IntegerKind::UnsignedLongLong
    });
    if !prototype || !character_pointer || *variadic != print || *result != expected {
        return Err(format!("unsupported owned runtime signature for {name}"));
    }
    Ok(print)
}
pub fn bridge(
    print: bool,
    formatter: Label,
    context: Symbol,
    labels: &mut LabelAllocator,
) -> Result<NativeFunction, String> {
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
