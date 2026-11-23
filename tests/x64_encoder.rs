use compiler::x64::*;
use std::collections::HashMap;
fn bytes(instructions: &[Instruction]) -> Vec<u8> {
    encode(instructions).unwrap().bytes
}
#[test]
fn reviewed_byte_fixtures() {
    assert_eq!(
        bytes(&[
            Instruction::LoadWord {
                dst: Reg::new(9).unwrap(),
                address: Address::base(Reg::new(12).unwrap(), 2)
            },
            Instruction::StoreWord {
                address: Address::base(Reg::new(13).unwrap(), 0),
                src: Reg::new(10).unwrap()
            },
            Instruction::Trap,
        ]),
        vec![
            0x45, 0x0f, 0xb7, 0x4c, 0x24, 0x02, 0x66, 0x45, 0x89, 0x55, 0x00, 0x0f, 0x0b
        ]
    );
    assert_eq!(
        bytes(&[
            Instruction::Shift {
                width: Width::Qword,
                op: Shift::Left,
                reg: Reg::new(9).unwrap(),
                count: ShiftCount::Immediate(13)
            },
            Instruction::Shift {
                width: Width::Dword,
                op: Shift::LogicalRight,
                reg: Reg::new(10).unwrap(),
                count: ShiftCount::Cl
            },
            Instruction::Shift {
                width: Width::Qword,
                op: Shift::ArithmeticRight,
                reg: Reg::new(12).unwrap(),
                count: ShiftCount::Immediate(1)
            },
            Instruction::DivideUnsigned {
                width: Width::Qword,
                divisor: Reg::new(15).unwrap()
            },
            Instruction::DivideUnsigned {
                width: Width::Dword,
                divisor: Reg::RCX
            },
        ]),
        vec![
            0x49, 0xc1, 0xe1, 0x0d, 0x41, 0xd3, 0xea, 0x49, 0xd1, 0xfc, 0x49, 0xf7, 0xf7, 0xf7,
            0xf1
        ]
    );
    assert_eq!(
        bytes(&[Instruction::MovImm {
            width: Width::Dword,
            dst: Reg::RAX,
            value: 42
        }]),
        vec![0xb8, 42, 0, 0, 0]
    );
    assert_eq!(
        bytes(&[Instruction::Mov {
            width: Width::Qword,
            dst: Reg::RAX,
            src: Reg::new(8).unwrap()
        }]),
        vec![0x4c, 0x89, 0xc0]
    );
    assert_eq!(
        bytes(&[Instruction::Load {
            width: Width::Qword,
            dst: Reg::RAX,
            address: Address::base(Reg::RSP, 8)
        }]),
        vec![0x48, 0x8b, 0x44, 0x24, 0x08]
    );
    assert_eq!(
        bytes(&[Instruction::Load {
            width: Width::Dword,
            dst: Reg::RAX,
            address: Address::base(Reg::new(13).unwrap(), 0)
        }]),
        vec![0x41, 0x8b, 0x45, 0]
    );
    assert_eq!(
        bytes(&[Instruction::Load {
            width: Width::Qword,
            dst: Reg::new(8).unwrap(),
            address: Address {
                base: Reg::new(12).unwrap(),
                index: Some((Reg::new(9).unwrap(), Scale::Four)),
                displacement: -16
            }
        }]),
        vec![0x4f, 0x8b, 0x44, 0x8c, 0xf0]
    );
    assert_eq!(
        bytes(&[Instruction::StoreByte {
            address: Address::base(Reg::RAX, 0),
            src: Reg::RSP
        }]),
        vec![0x40, 0x88, 0x20]
    );
    assert_eq!(
        bytes(&[
            Instruction::SignExtend(Width::Dword),
            Instruction::Divide {
                width: Width::Dword,
                divisor: Reg::new(8).unwrap()
            }
        ]),
        vec![0x99, 0x41, 0xf7, 0xf8]
    );
    assert_eq!(
        bytes(&[
            Instruction::Push(Reg::new(15).unwrap()),
            Instruction::Pop(Reg::new(15).unwrap()),
            Instruction::Ret
        ]),
        vec![0x41, 0x57, 0x41, 0x5f, 0xc3]
    );
}

#[test]
#[cfg(all(windows, target_arch = "x86_64"))]
fn word_stores_do_not_clobber_adjacent_bytes() {
    run(
        vec![
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::RAX,
                value: 0x1122334455667788,
            },
            Instruction::Store {
                width: Width::Qword,
                address: Address::base(Reg::RSP, 8),
                src: Reg::RAX,
            },
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::new(10).unwrap(),
                value: 0xabcd,
            },
            Instruction::StoreWord {
                address: Address::base(Reg::RSP, 10),
                src: Reg::new(10).unwrap(),
            },
            Instruction::Load {
                width: Width::Qword,
                dst: Reg::RAX,
                address: Address::base(Reg::RSP, 8),
            },
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::new(10).unwrap(),
                value: 0x11223344abcd7788,
            },
            Instruction::Binary {
                width: Width::Qword,
                op: Binary::Compare,
                dst: Reg::RAX,
                src: Reg::new(10).unwrap(),
            },
            Instruction::Set {
                condition: Condition::Equal,
                dst: Reg::RAX,
            },
        ],
        1,
    );
    run(
        vec![
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::RAX,
                value: 0x1122334455667788,
            },
            Instruction::Store {
                width: Width::Qword,
                address: Address::base(Reg::RSP, 8),
                src: Reg::RAX,
            },
            Instruction::LoadWord {
                dst: Reg::RAX,
                address: Address::base(Reg::RSP, 10),
            },
        ],
        0x5566,
    );
}

#[test]
#[cfg(all(windows, target_arch = "x86_64"))]
fn native_unsigned_division_and_shift_boundaries() {
    for (width, bits) in [(Width::Dword, 32), (Width::Qword, 64)] {
        for (shift, expected) in [
            (Shift::Left, 1u64 << (bits - 1)),
            (Shift::LogicalRight, 1),
            (Shift::ArithmeticRight, compiler::types::mask(bits)),
        ] {
            let input = if shift == Shift::Left {
                1
            } else {
                1u64 << (bits - 1)
            };
            for count in [ShiftCount::Immediate((bits - 1) as u8), ShiftCount::Cl] {
                run(
                    vec![
                        Instruction::MovImm {
                            width,
                            dst: Reg::RAX,
                            value: input,
                        },
                        Instruction::MovImm {
                            width: Width::Dword,
                            dst: Reg::RCX,
                            value: u64::from(bits - 1),
                        },
                        Instruction::Shift {
                            width,
                            op: shift,
                            reg: Reg::RAX,
                            count,
                        },
                        Instruction::MovImm {
                            width,
                            dst: Reg::new(10).unwrap(),
                            value: expected,
                        },
                        Instruction::Binary {
                            width,
                            op: Binary::Compare,
                            dst: Reg::RAX,
                            src: Reg::new(10).unwrap(),
                        },
                        Instruction::Set {
                            condition: Condition::Equal,
                            dst: Reg::RAX,
                        },
                    ],
                    1,
                );
            }
        }
        for remainder in [false, true] {
            let mut body = vec![
                Instruction::MovImm {
                    width,
                    dst: Reg::RAX,
                    value: compiler::types::mask(bits),
                },
                Instruction::MovImm {
                    width,
                    dst: Reg::RDX,
                    value: 0,
                },
                Instruction::MovImm {
                    width,
                    dst: Reg::new(10).unwrap(),
                    value: 2,
                },
                Instruction::DivideUnsigned {
                    width,
                    divisor: Reg::new(10).unwrap(),
                },
            ];
            if remainder {
                body.push(Instruction::Mov {
                    width,
                    dst: Reg::RAX,
                    src: Reg::RDX,
                });
            }
            body.extend([
                Instruction::MovImm {
                    width,
                    dst: Reg::new(10).unwrap(),
                    value: if remainder {
                        1
                    } else {
                        compiler::types::mask(bits) / 2
                    },
                },
                Instruction::Binary {
                    width,
                    op: Binary::Compare,
                    dst: Reg::RAX,
                    src: Reg::new(10).unwrap(),
                },
                Instruction::Set {
                    condition: Condition::Equal,
                    dst: Reg::RAX,
                },
            ]);
            run(body, 1);
        }
    }
}
#[test]
fn labels_symbols_and_ranges() {
    assert_eq!(
        bytes(&[
            Instruction::Jump(Label(1)),
            Instruction::Nop,
            Instruction::Bind(Label(1)),
            Instruction::Ret
        ]),
        vec![0xe9, 1, 0, 0, 0, 0x90, 0xc3]
    );
    assert_eq!(
        bytes(&[Instruction::Bind(Label(0)), Instruction::Jump(Label(0))]),
        vec![0xe9, 0xfb, 0xff, 0xff, 0xff]
    );
    assert!(encode(&[Instruction::Jump(Label(0))]).is_err());
    assert!(encode(&[Instruction::Bind(Label(0)), Instruction::Bind(Label(0))]).is_err());
    assert!(Reg::new(16).is_err());
    assert!(
        encode(&[Instruction::MovImm {
            width: Width::Dword,
            dst: Reg::RAX,
            value: u64::MAX
        }])
        .is_err()
    );
    assert!(
        encode(&[Instruction::Load {
            width: Width::Qword,
            dst: Reg::RAX,
            address: Address {
                base: Reg::RAX,
                index: Some((Reg::RSP, Scale::One)),
                displacement: 0
            }
        }])
        .is_err()
    );
    for (target, next, expected) in [(2147483647, 0, i32::MAX), (0, 2147483648, i32::MIN)] {
        assert_eq!(relative32(target, next).unwrap(), expected);
    }
    assert!(relative32(2147483648, 0).is_err());
    assert!(relative32(0, 2147483649).is_err());
    assert!(relative32(u64::MAX, 0).is_err());
    let fragment = encode(&[Instruction::CallImport(Symbol(0))]).unwrap();
    assert!(fragment.resolve(4096, &HashMap::new()).is_err());
    assert_eq!(
        fragment
            .resolve(4096, &HashMap::from([(Symbol(0), 8272)]))
            .unwrap(),
        vec![0xff, 0x15, 0x4a, 0x10, 0, 0]
    );
    assert!(fragment.dump().contains("00000000 +06 CallImport"));
    let mut corrupt = fragment;
    corrupt.external_fixups[0].displacement_offset = usize::MAX;
    assert!(
        corrupt
            .resolve(0, &HashMap::from([(Symbol(0), 1)]))
            .is_err()
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
fn native(instructions: &[Instruction], expected: i32) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ID: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "encoder execution {} {}.exe",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let fragment = encode(instructions).unwrap();
    let mut symbols = HashMap::from([(Symbol(0), 8272)]);
    if let Some(row) = fragment
        .listing
        .iter()
        .find(|r| matches!(r.instruction, Instruction::Bind(Label(1))))
    {
        symbols.insert(Symbol(1), 4096 + row.offset as u32);
    }
    let code = fragment.resolve(4096, &symbols).unwrap();
    std::fs::write(&path, compiler::native_pe::image(&code).unwrap()).unwrap();
    let mut child = std::process::Command::new(&path).spawn().unwrap();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if start.elapsed().as_secs() >= 5 {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native encoder test timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    std::fs::remove_file(path).unwrap();
    assert_eq!(status.code(), Some(expected));
}
#[cfg(all(windows, target_arch = "x86_64"))]
fn run(mut body: Vec<Instruction>, expected: i32) {
    let mut c = vec![Instruction::Immediate {
        width: Width::Qword,
        op: Immediate::Sub,
        reg: Reg::RSP,
        value: 40,
    }];
    c.append(&mut body);
    c.push(Instruction::Mov {
        width: Width::Dword,
        dst: Reg::RCX,
        src: Reg::RAX,
    });
    c.push(Instruction::CallImport(Symbol(0)));
    native(&c, expected);
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn native_arithmetic_conditions_and_division() {
    for (op, expected) in [
        (Binary::Add, 37),
        (Binary::Sub, 23),
        (Binary::And, 6),
        (Binary::Or, 31),
        (Binary::Xor, 25),
    ] {
        run(
            vec![
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: 30,
                },
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::new(10).unwrap(),
                    value: 7,
                },
                Instruction::Binary {
                    width: Width::Dword,
                    op,
                    dst: Reg::RAX,
                    src: Reg::new(10).unwrap(),
                },
            ],
            expected,
        );
    }
    run(
        vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 6,
            },
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::new(9).unwrap(),
                value: 7,
            },
            Instruction::Multiply {
                width: Width::Dword,
                dst: Reg::RAX,
                src: Reg::new(9).unwrap(),
            },
        ],
        42,
    );
    for condition in [
        Condition::Less,
        Condition::GreaterEqual,
        Condition::Equal,
        Condition::NotEqual,
    ] {
        run(
            vec![
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: 3,
                },
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RDX,
                    value: 7,
                },
                Instruction::Binary {
                    width: Width::Dword,
                    op: Binary::Compare,
                    dst: Reg::RAX,
                    src: Reg::RDX,
                },
                Instruction::Set {
                    condition,
                    dst: Reg::new(8).unwrap(),
                },
                Instruction::Mov {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    src: Reg::new(8).unwrap(),
                },
            ],
            if matches!(condition, Condition::Less | Condition::NotEqual) {
                1
            } else {
                0
            },
        );
    }
    for rem in [false, true] {
        let mut body = vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: (-17i32 as u32) as u64,
            },
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::new(10).unwrap(),
                value: 5,
            },
            Instruction::SignExtend(Width::Dword),
            Instruction::Divide {
                width: Width::Dword,
                divisor: Reg::new(10).unwrap(),
            },
        ];
        if rem {
            body.push(Instruction::Mov {
                width: Width::Dword,
                dst: Reg::RAX,
                src: Reg::RDX,
            });
        }
        run(body, if rem { -2 } else { -3 });
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn native_memory_and_calls() {
    for base in [
        Reg::RSP,
        Reg::RBP,
        Reg::new(12).unwrap(),
        Reg::new(13).unwrap(),
    ] {
        let mut b = vec![];
        if base != Reg::RSP {
            b.push(Instruction::Mov {
                width: Width::Qword,
                dst: base,
                src: Reg::RSP,
            });
        }
        b.extend([
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::new(10).unwrap(),
                value: 42,
            },
            Instruction::Store {
                width: Width::Qword,
                address: Address::base(base, 8),
                src: Reg::new(10).unwrap(),
            },
            Instruction::Load {
                width: Width::Qword,
                dst: Reg::RAX,
                address: Address::base(base, 8),
            },
        ]);
        run(b, 42);
    }
    run(
        vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 255,
            },
            Instruction::StoreByte {
                address: Address::base(Reg::RSP, 8),
                src: Reg::RAX,
            },
            Instruction::LoadByte {
                dst: Reg::RAX,
                address: Address::base(Reg::RSP, 8),
            },
        ],
        255,
    );
    native(
        &[
            Instruction::Immediate {
                width: Width::Qword,
                op: Immediate::Sub,
                reg: Reg::RSP,
                value: 40,
            },
            Instruction::Call(Label(1)),
            Instruction::Mov {
                width: Width::Dword,
                dst: Reg::RCX,
                src: Reg::RAX,
            },
            Instruction::CallImport(Symbol(0)),
            Instruction::Bind(Label(1)),
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 42,
            },
            Instruction::Ret,
        ],
        42,
    );
    run(
        vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 0,
            },
            Instruction::Bind(Label(1)),
            Instruction::Immediate {
                width: Width::Dword,
                op: Immediate::Add,
                reg: Reg::RAX,
                value: 1,
            },
            Instruction::Immediate {
                width: Width::Dword,
                op: Immediate::Compare,
                reg: Reg::RAX,
                value: 5,
            },
            Instruction::Branch {
                condition: Condition::Less,
                label: Label(1),
            },
        ],
        5,
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn native_extended_registers_indexing_and_qword_math() {
    let r8 = Reg::new(8).unwrap();
    let r10 = Reg::new(10).unwrap();
    for op in [Binary::Add, Binary::Sub, Binary::Xor] {
        let (a, b, expected) = match op {
            Binary::Add => (0x100000002, 0x100000003, 0x200000005),
            Binary::Sub => (0x200000008, 0x100000003, 0x100000005),
            _ => (0x100000002, 0x200000003, 0x300000001),
        };
        run(
            vec![
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: Reg::RAX,
                    value: a,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: r10,
                    value: b,
                },
                Instruction::Binary {
                    width: Width::Qword,
                    op,
                    dst: Reg::RAX,
                    src: r10,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: r10,
                    value: expected,
                },
                Instruction::Binary {
                    width: Width::Qword,
                    op: Binary::Compare,
                    dst: Reg::RAX,
                    src: r10,
                },
                Instruction::Set {
                    condition: Condition::Equal,
                    dst: Reg::RAX,
                },
            ],
            1,
        );
    }
    run(
        vec![
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::RAX,
                value: (-17i64) as u64,
            },
            Instruction::MovImm {
                width: Width::Qword,
                dst: r10,
                value: 5,
            },
            Instruction::SignExtend(Width::Qword),
            Instruction::Divide {
                width: Width::Qword,
                divisor: r10,
            },
        ],
        -3,
    );
    let address = Address {
        base: Reg::RSP,
        index: Some((r8, Scale::Four)),
        displacement: 8,
    };
    run(
        vec![
            Instruction::MovImm {
                width: Width::Qword,
                dst: r8,
                value: 2,
            },
            Instruction::MovImm {
                width: Width::Dword,
                dst: r10,
                value: 77,
            },
            Instruction::Store {
                width: Width::Dword,
                address,
                src: r10,
            },
            Instruction::Lea { dst: r10, address },
            Instruction::Load {
                width: Width::Dword,
                dst: Reg::RAX,
                address: Address::base(r10, 0),
            },
        ],
        77,
    );
    for src in [
        Reg::RBP,
        Reg::new(6).unwrap(),
        Reg::new(7).unwrap(),
        Reg::new(15).unwrap(),
    ] {
        run(
            vec![
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: src,
                    value: 203,
                },
                Instruction::StoreByte {
                    address: Address::base(Reg::RSP, 8),
                    src,
                },
                Instruction::LoadByte {
                    dst: Reg::RAX,
                    address: Address::base(Reg::RSP, 8),
                },
            ],
            203,
        );
    }
    for dst in [
        Reg::RBP,
        Reg::new(6).unwrap(),
        Reg::new(7).unwrap(),
        Reg::new(15).unwrap(),
    ] {
        run(
            vec![
                Instruction::MovImm {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    value: 7,
                },
                Instruction::Binary {
                    width: Width::Dword,
                    op: Binary::Compare,
                    dst: Reg::RAX,
                    src: Reg::RAX,
                },
                Instruction::Set {
                    condition: Condition::Equal,
                    dst,
                },
                Instruction::Mov {
                    width: Width::Dword,
                    dst: Reg::RAX,
                    src: dst,
                },
            ],
            1,
        );
    }
    native(
        &[
            Instruction::Immediate {
                width: Width::Qword,
                op: Immediate::Sub,
                reg: Reg::RSP,
                value: 40,
            },
            Instruction::LeaSymbol {
                dst: r10,
                symbol: Symbol(1),
            },
            Instruction::CallReg(r10),
            Instruction::Mov {
                width: Width::Dword,
                dst: Reg::RCX,
                src: Reg::RAX,
            },
            Instruction::CallImport(Symbol(0)),
            Instruction::Bind(Label(1)),
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 91,
            },
            Instruction::Ret,
        ],
        91,
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn native_every_condition_and_stack_register_ops() {
    for (condition, expected) in [
        (Condition::Overflow, 0),
        (Condition::NotOverflow, 1),
        (Condition::Below, 1),
        (Condition::AboveEqual, 0),
        (Condition::Equal, 0),
        (Condition::NotEqual, 1),
        (Condition::BelowEqual, 1),
        (Condition::Above, 0),
        (Condition::Sign, 1),
        (Condition::NotSign, 0),
        (Condition::Less, 1),
        (Condition::GreaterEqual, 0),
        (Condition::LessEqual, 1),
        (Condition::Greater, 0),
    ] {
        let mut c = vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 3,
            },
            Instruction::Immediate {
                width: Width::Dword,
                op: Immediate::Compare,
                reg: Reg::RAX,
                value: 7,
            },
        ];
        c.extend([
            Instruction::Branch {
                condition,
                label: Label(2),
            },
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 0,
            },
            Instruction::Jump(Label(3)),
            Instruction::Bind(Label(2)),
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: 1,
            },
            Instruction::Bind(Label(3)),
        ]);
        run(c, expected);
    }
    run(
        vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: Reg::RAX,
                value: i32::MAX as u64,
            },
            Instruction::Immediate {
                width: Width::Dword,
                op: Immediate::Add,
                reg: Reg::RAX,
                value: 1,
            },
            Instruction::Set {
                condition: Condition::Overflow,
                dst: Reg::RAX,
            },
        ],
        1,
    );
    let r15 = Reg::new(15).unwrap();
    run(
        vec![
            Instruction::MovImm {
                width: Width::Dword,
                dst: r15,
                value: 42,
            },
            Instruction::Push(r15),
            Instruction::MovImm {
                width: Width::Dword,
                dst: r15,
                value: 0,
            },
            Instruction::Pop(r15),
            Instruction::Mov {
                width: Width::Dword,
                dst: Reg::RAX,
                src: r15,
            },
            Instruction::Negate {
                width: Width::Dword,
                reg: Reg::RAX,
            },
        ],
        -42,
    );
}
