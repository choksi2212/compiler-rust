//! Development oracle input generator. Not used by the production compiler.
use compiler::x64::*;
use std::io::Write;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("expected fixture output path")?;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let q = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    let d = [
        "eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d",
        "r12d", "r13d", "r14d", "r15d",
    ];
    let byte = [
        "al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b",
        "r13b", "r14b", "r15b",
    ];
    let mut count = 0;
    let mut emit = |instruction, asm: String| -> Result<(), Box<dyn std::error::Error>> {
        let f = encode(&[instruction])?;
        let hex = f
            .bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        writeln!(file, "{hex}\t{asm}")?;
        count += 1;
        Ok(())
    };
    for (width, suffix, names) in [(Width::Dword, "l", &d), (Width::Qword, "q", &q)] {
        for (reg, name) in names.iter().enumerate() {
            let r = Reg::new(reg as u8)?;
            emit(
                Instruction::DivideUnsigned { width, divisor: r },
                format!("div{suffix} %{}", name),
            )?;
            for (op, mnemonic) in [
                (Shift::Left, "shl"),
                (Shift::LogicalRight, "shr"),
                (Shift::ArithmeticRight, "sar"),
            ] {
                emit(
                    Instruction::Shift {
                        width,
                        op,
                        reg: r,
                        count: ShiftCount::Cl,
                    },
                    format!("{mnemonic}{suffix} %cl,%{}", name),
                )?;
                for amount in [0, 1, 7, 8, 15, 16, 31, 32, 63, 64, 127, 255] {
                    emit(
                        Instruction::Shift {
                            width,
                            op,
                            reg: r,
                            count: ShiftCount::Immediate(amount),
                        },
                        format!("{mnemonic}{suffix} ${amount},%{}", name),
                    )?;
                }
            }
        }
    }
    for (width, suffix, names) in [(Width::Dword, "l", &d), (Width::Qword, "q", &q)] {
        for dst in 0..16 {
            let r = Reg::new(dst as u8)?;
            emit(
                Instruction::MovImm {
                    width,
                    dst: r,
                    value: 0x12345678,
                },
                format!(
                    "{} $0x12345678,%{}",
                    if width == Width::Qword {
                        "movabsq"
                    } else {
                        "movl"
                    },
                    names[dst]
                ),
            )?;
            for src in 0..16 {
                let a = Reg::new(src as u8)?;
                emit(
                    Instruction::Mov {
                        width,
                        dst: r,
                        src: a,
                    },
                    format!("mov{suffix} %{},%{}", names[src], names[dst]),
                )?;
                for (op, mnemonic) in [
                    (Binary::Add, "add"),
                    (Binary::Sub, "sub"),
                    (Binary::And, "and"),
                    (Binary::Or, "or"),
                    (Binary::Xor, "xor"),
                    (Binary::Compare, "cmp"),
                    (Binary::Test, "test"),
                ] {
                    emit(
                        Instruction::Binary {
                            width,
                            op,
                            dst: r,
                            src: a,
                        },
                        format!("{mnemonic}{suffix} %{},%{}", names[src], names[dst]),
                    )?;
                }
                emit(
                    Instruction::Multiply {
                        width,
                        dst: r,
                        src: a,
                    },
                    format!("imul{suffix} %{},%{}", names[src], names[dst]),
                )?;
            }
            emit(
                Instruction::Negate { width, reg: r },
                format!("neg{suffix} %{}", names[dst]),
            )?;
            emit(
                Instruction::Divide { width, divisor: r },
                format!("idiv{suffix} %{}", names[dst]),
            )?;
            for (op, mnemonic) in [
                (Immediate::Add, "add"),
                (Immediate::Sub, "sub"),
                (Immediate::Compare, "cmp"),
            ] {
                emit(
                    Instruction::Immediate {
                        width,
                        op,
                        reg: r,
                        value: 0x12345678,
                    },
                    format!("{mnemonic}{suffix} $0x12345678,%{}", names[dst]),
                )?;
            }
            for (base, base_name) in q.iter().enumerate() {
                for displacement in [-129, -128, -1, 0, 127, 128, i32::MIN, i32::MAX] {
                    let address = Address::base(Reg::new(base as u8)?, displacement);
                    let mem = format!("{displacement}(%{})", base_name);
                    emit(
                        Instruction::Load {
                            width,
                            dst: r,
                            address,
                        },
                        format!("mov{suffix} {mem},%{}", names[dst]),
                    )?;
                    emit(
                        Instruction::Store {
                            width,
                            address,
                            src: r,
                        },
                        format!("mov{suffix} %{},{}", names[dst], mem),
                    )?;
                }
            }
        }
    }
    for (base, base_name) in q.iter().enumerate() {
        for (index, index_name) in q.iter().enumerate() {
            if index == 4 {
                continue;
            }
            for (scale, n) in [
                (Scale::One, 1),
                (Scale::Two, 2),
                (Scale::Four, 4),
                (Scale::Eight, 8),
            ] {
                let address = Address {
                    base: Reg::new(base as u8)?,
                    index: Some((Reg::new(index as u8)?, scale)),
                    displacement: 128,
                };
                emit(
                    Instruction::Lea {
                        dst: Reg::RAX,
                        address,
                    },
                    format!("leaq 128(%{},%{},{n}),%rax", base_name, index_name),
                )?;
            }
        }
    }
    for r in 0..16 {
        let reg = Reg::new(r as u8)?;
        emit(Instruction::Push(reg), format!("pushq %{}", q[r]))?;
        emit(Instruction::Pop(reg), format!("popq %{}", q[r]))?;
        emit(Instruction::CallReg(reg), format!("call *%{}", q[r]))?;
        for (condition, mnemonic) in [
            (Condition::Equal, "e"),
            (Condition::NotEqual, "ne"),
            (Condition::Less, "l"),
            (Condition::Greater, "g"),
            (Condition::Overflow, "o"),
            (Condition::Below, "b"),
            (Condition::NotOverflow, "no"),
            (Condition::AboveEqual, "ae"),
            (Condition::BelowEqual, "be"),
            (Condition::Above, "a"),
            (Condition::Sign, "s"),
            (Condition::NotSign, "ns"),
            (Condition::GreaterEqual, "ge"),
            (Condition::LessEqual, "le"),
        ] {
            emit(
                Instruction::Set {
                    condition,
                    dst: reg,
                },
                format!("set{mnemonic} %{}; movzbl %{},%{}", byte[r], byte[r], d[r]),
            )?;
        }
        for (base, base_name) in q.iter().enumerate() {
            let address = Address::base(Reg::new(base as u8)?, 0);
            emit(
                Instruction::LoadByte { dst: reg, address },
                format!("movzbl (%{}),%{}", base_name, d[r]),
            )?;
            emit(
                Instruction::StoreByte { address, src: reg },
                format!("movb %{},(%{})", byte[r], base_name),
            )?;
        }
    }
    emit(Instruction::SignExtend(Width::Dword), "cltd".into())?;
    emit(Instruction::SignExtend(Width::Qword), "cqto".into())?;
    emit(Instruction::Ret, "ret".into())?;
    emit(Instruction::Nop, "nop".into())?;
    println!("Encoded {count} oracle fixtures");
    Ok(())
}
