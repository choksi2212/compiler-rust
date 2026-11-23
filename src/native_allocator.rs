//! Owned allocation metadata and Windows virtual-memory-backed C services.
use crate::{
    abi::{FramePlan, FrameRequest},
    native_integer::NativeFunction,
    x64::{
        Address, Binary, Condition, Immediate, Instruction as M, Label, LabelAllocator, Reg,
        Symbol, Width,
    },
};
fn r(n: u8) -> Reg {
    Reg::new(n).unwrap()
}
fn imm(dst: Reg, value: u64) -> M {
    M::MovImm {
        width: Width::Qword,
        dst,
        value,
    }
}
fn mov(dst: Reg, src: Reg) -> M {
    M::Mov {
        width: Width::Qword,
        dst,
        src,
    }
}
fn load(dst: Reg, base: Reg, offset: i32) -> M {
    M::Load {
        width: Width::Qword,
        dst,
        address: Address::base(base, offset),
    }
}
fn store(base: Reg, offset: i32, src: Reg) -> M {
    M::Store {
        width: Width::Qword,
        address: Address::base(base, offset),
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
fn cmp(a: Reg, b: Reg) -> M {
    M::Binary {
        width: Width::Qword,
        op: Binary::Compare,
        dst: a,
        src: b,
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
fn branch(condition: Condition, label: Label) -> M {
    M::Branch { condition, label }
}
fn finish(mut code: Vec<M>, frame: FramePlan, unwind: Vec<u8>) -> NativeFunction {
    code.extend(frame.epilogue());
    NativeFunction {
        instructions: code,
        frame,
        unwind,
    }
}
pub fn malloc(allocate: Symbol, labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        saved: vec![r(3)],
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let failed = labels.fresh()?;
    let done = labels.fresh()?;
    code.extend([
        mov(r(3), Reg::RCX),
        test(r(3)),
        branch(Condition::Equal, failed),
        imm(r(10), u64::MAX - 16),
        cmp(r(3), r(10)),
        branch(Condition::Above, failed),
        mov(Reg::RDX, r(3)),
        change(Reg::RDX, 16),
        imm(Reg::RCX, 0),
        imm(r(8), 0x3000),
        imm(r(9), 4),
        M::CallImport(allocate),
        test(Reg::RAX),
        branch(Condition::Equal, done),
        store(Reg::RAX, 0, r(3)),
        imm(r(10), 0),
        store(Reg::RAX, 8, r(10)),
        change(Reg::RAX, 16),
        M::Jump(done),
        M::Bind(failed),
        imm(Reg::RAX, 0),
        M::Bind(done),
    ]);
    Ok(finish(code, frame, prologue.unwind))
}
pub fn free(release: Symbol, labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let done = labels.fresh()?;
    code.extend([
        test(Reg::RCX),
        branch(Condition::Equal, done),
        change(Reg::RCX, -16),
        imm(Reg::RDX, 0),
        imm(r(8), 0x8000),
        M::CallImport(release),
        M::Bind(done),
    ]);
    Ok(finish(code, frame, prologue.unwind))
}
pub fn calloc(allocate: Label, labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        saved: vec![r(3), r(6)],
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let failed = labels.fresh()?;
    let done = labels.fresh()?;
    code.extend([
        mov(r(3), Reg::RCX),
        mov(r(6), Reg::RDX),
        test(r(3)),
        branch(Condition::Equal, failed),
        test(r(6)),
        branch(Condition::Equal, failed),
        imm(Reg::RAX, u64::MAX - 16),
        imm(Reg::RDX, 0),
        M::DivideUnsigned {
            width: Width::Qword,
            divisor: r(3),
        },
        cmp(r(6), Reg::RAX),
        branch(Condition::Above, failed),
        M::Multiply {
            width: Width::Qword,
            dst: r(3),
            src: r(6),
        },
        mov(Reg::RCX, r(3)),
        M::Call(allocate),
        M::Jump(done),
        M::Bind(failed),
        imm(Reg::RAX, 0),
        M::Bind(done),
    ]);
    Ok(finish(code, frame, prologue.unwind))
}
pub fn realloc(
    allocate: Label,
    release: Label,
    labels: &mut LabelAllocator,
) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        saved: vec![r(3), r(6), r(7), r(12)],
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let nonnull = labels.fresh()?;
    let nonzero = labels.fresh()?;
    let sized = labels.fresh()?;
    let again = labels.fresh()?;
    let copied = labels.fresh()?;
    let done = labels.fresh()?;
    code.extend([
        mov(r(3), Reg::RCX),
        mov(r(6), Reg::RDX),
        test(r(3)),
        branch(Condition::NotEqual, nonnull),
        mov(Reg::RCX, r(6)),
        M::Call(allocate),
        M::Jump(done),
        M::Bind(nonnull),
        test(r(6)),
        branch(Condition::NotEqual, nonzero),
        mov(Reg::RCX, r(3)),
        M::Call(release),
        imm(Reg::RAX, 0),
        M::Jump(done),
        M::Bind(nonzero),
        load(r(12), r(3), -16),
        cmp(r(12), r(6)),
        branch(Condition::BelowEqual, sized),
        mov(r(12), r(6)),
        M::Bind(sized),
        mov(Reg::RCX, r(6)),
        M::Call(allocate),
        test(Reg::RAX),
        branch(Condition::Equal, done),
        mov(r(7), Reg::RAX),
        mov(r(10), r(3)),
        mov(r(11), r(7)),
        test(r(12)),
        branch(Condition::Equal, copied),
        M::Bind(again),
        M::LoadByte {
            dst: Reg::RAX,
            address: Address::base(r(10), 0),
        },
        M::StoreByte {
            address: Address::base(r(11), 0),
            src: Reg::RAX,
        },
        change(r(10), 1),
        change(r(11), 1),
        change(r(12), -1),
        test(r(12)),
        branch(Condition::NotEqual, again),
        M::Bind(copied),
        mov(Reg::RCX, r(3)),
        M::Call(release),
        mov(Reg::RAX, r(7)),
        M::Bind(done),
    ]);
    Ok(finish(code, frame, prologue.unwind))
}
