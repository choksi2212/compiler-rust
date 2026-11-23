//! Byte-oriented owned C memory helpers, emitted as native Windows ABI code.
use crate::{
    abi::{FramePlan, FrameRequest},
    native_integer::NativeFunction,
    typed_runtime::Service,
    x64::{Address, Binary, Condition, Immediate, Instruction as M, LabelAllocator, Reg, Width},
};
fn mov(dst: Reg, src: Reg) -> M {
    M::Mov {
        width: Width::Qword,
        dst,
        src,
    }
}
fn change(reg: Reg, value: i32) -> M {
    M::Immediate {
        width: Width::Qword,
        op: Immediate::Add,
        reg,
        value,
    }
}
fn test(reg: Reg) -> M {
    M::Binary {
        width: Width::Qword,
        op: Binary::Test,
        dst: reg,
        src: reg,
    }
}
pub fn bridge(service: Service, labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest::default())?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let r8 = Reg::new(8)?;
    let r10 = Reg::new(10)?;
    let r11 = Reg::new(11)?;
    let again = labels.fresh()?;
    let done = labels.fresh()?;
    code.push(mov(Reg::RAX, Reg::RCX));
    match service {
        Service::Memcpy | Service::Memmove | Service::Memset => {
            code.extend([
                test(r8),
                M::Branch {
                    condition: Condition::Equal,
                    label: done,
                },
            ]);
            if service == Service::Memmove {
                let forward = labels.fresh()?;
                let backward = labels.fresh()?;
                code.extend([
                    M::Binary {
                        width: Width::Qword,
                        op: Binary::Compare,
                        dst: Reg::RCX,
                        src: Reg::RDX,
                    },
                    M::Branch {
                        condition: Condition::BelowEqual,
                        label: forward,
                    },
                    M::Binary {
                        width: Width::Qword,
                        op: Binary::Add,
                        dst: Reg::RCX,
                        src: r8,
                    },
                    M::Binary {
                        width: Width::Qword,
                        op: Binary::Add,
                        dst: Reg::RDX,
                        src: r8,
                    },
                    M::Bind(backward),
                    change(Reg::RCX, -1),
                    change(Reg::RDX, -1),
                    M::LoadByte {
                        dst: r10,
                        address: Address::base(Reg::RDX, 0),
                    },
                    M::StoreByte {
                        address: Address::base(Reg::RCX, 0),
                        src: r10,
                    },
                    change(r8, -1),
                    test(r8),
                    M::Branch {
                        condition: Condition::NotEqual,
                        label: backward,
                    },
                    M::Jump(done),
                    M::Bind(forward),
                ]);
            }
            code.push(M::Bind(again));
            if service != Service::Memset {
                code.extend([
                    M::LoadByte {
                        dst: r10,
                        address: Address::base(Reg::RDX, 0),
                    },
                    change(Reg::RDX, 1),
                ]);
            }
            code.extend([
                M::StoreByte {
                    address: Address::base(Reg::RCX, 0),
                    src: if service == Service::Memset {
                        Reg::RDX
                    } else {
                        r10
                    },
                },
                change(Reg::RCX, 1),
                change(r8, -1),
                test(r8),
                M::Branch {
                    condition: Condition::NotEqual,
                    label: again,
                },
            ]);
        }
        Service::Strcpy => {
            code.extend([
                M::Bind(again),
                M::LoadByte {
                    dst: r10,
                    address: Address::base(Reg::RDX, 0),
                },
                M::StoreByte {
                    address: Address::base(Reg::RCX, 0),
                    src: r10,
                },
                change(Reg::RCX, 1),
                change(Reg::RDX, 1),
                test(r10),
                M::Branch {
                    condition: Condition::NotEqual,
                    label: again,
                },
            ]);
        }
        Service::Strcmp => {
            let different = labels.fresh()?;
            code.extend([
                M::Bind(again),
                M::LoadByte {
                    dst: r10,
                    address: Address::base(Reg::RCX, 0),
                },
                M::LoadByte {
                    dst: r11,
                    address: Address::base(Reg::RDX, 0),
                },
                M::Binary {
                    width: Width::Dword,
                    op: Binary::Compare,
                    dst: r10,
                    src: r11,
                },
                M::Branch {
                    condition: Condition::NotEqual,
                    label: different,
                },
                test(r10),
                M::Branch {
                    condition: Condition::Equal,
                    label: different,
                },
                change(Reg::RCX, 1),
                change(Reg::RDX, 1),
                M::Jump(again),
                M::Bind(different),
                mov(Reg::RAX, r10),
                M::Binary {
                    width: Width::Dword,
                    op: Binary::Sub,
                    dst: Reg::RAX,
                    src: r11,
                },
            ]);
        }
        _ => return Err("memory bridge requires a memory/string service".into()),
    }
    code.push(M::Bind(done));
    code.extend(frame.epilogue());
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
