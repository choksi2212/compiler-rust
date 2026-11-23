use compiler::pe::*;
use compiler::x64::*;
fn spec(instructions: &[Instruction]) -> ImageSpec {
    ImageSpec {
        text: encode(instructions).unwrap(),
        code_symbols: vec![],
        data: vec![],
        imports: vec![Import {
            symbol: Symbol(0),
            dll: "kernel32.dll".into(),
            name: "ExitProcess".into(),
        }],
        pointers: vec![],
        entry_offset: 0,
        image_base: 0x140000000,
        exception_directory: None,
    }
}
fn exit(code: u64) -> Vec<Instruction> {
    vec![
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Sub,
            reg: Reg::RSP,
            value: 40,
        },
        Instruction::MovImm {
            width: Width::Dword,
            dst: Reg::RCX,
            value: code,
        },
        Instruction::CallImport(Symbol(0)),
    ]
}
fn dword(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
#[test]
fn dynamic_layout_is_deterministic_and_exceeds_foundation() {
    let mut c = vec![Instruction::Nop; 5000];
    c.extend(exit(37));
    let mut s = spec(&c);
    s.data.push(DataSection {
        name: ".rdata".into(),
        bytes: vec![0x55; 9000],
        virtual_size: 9000,
        access: Access::ReadOnly,
        symbols: vec![(Symbol(1), 0)],
    });
    s.data.push(DataSection {
        name: ".bss".into(),
        bytes: vec![],
        virtual_size: 8192,
        access: Access::ZeroFill,
        symbols: vec![(Symbol(2), 0)],
    });
    let a = build(&s).unwrap();
    let b = build(&s).unwrap();
    assert_eq!(a.bytes, b.bytes);
    validate(&a.bytes).unwrap();
    assert_eq!(a.sections.len(), 5);
    assert!(a.sections[0].virtual_size > 4096);
    assert_eq!(a.sections[2].raw_size, 0);
    assert_eq!(a.sections[2].raw_offset, 0);
    assert_eq!(dword(&a.bytes, 152 + 56) % 4096, 0);
    for section in &a.sections {
        assert_eq!(section.rva % 4096, 0);
        assert_eq!(section.raw_size % 512, 0);
        if section.raw_size > 0 {
            assert_eq!(section.raw_offset % 512, 0);
            assert!(section.raw_offset as usize + section.raw_size as usize <= a.bytes.len());
        }
    }
}
#[test]
fn import_aliases_and_input_order() {
    let mut s = spec(&exit(0));
    s.imports.extend([
        Import {
            symbol: Symbol(1),
            dll: "KERNEL32.DLL".into(),
            name: "ExitProcess".into(),
        },
        Import {
            symbol: Symbol(2),
            dll: "kernel32.dll".into(),
            name: "GetCurrentProcessId".into(),
        },
        Import {
            symbol: Symbol(3),
            dll: "user32.dll".into(),
            name: "GetDoubleClickTime".into(),
        },
    ]);
    let a = build(&s).unwrap();
    s.imports.reverse();
    let b = build(&s).unwrap();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.symbols[&Symbol(0)], a.symbols[&Symbol(1)]);
    assert_ne!(a.symbols[&Symbol(0)], a.symbols[&Symbol(2)]);
}
#[test]
fn invalid_specs_rejected() {
    let base = spec(&exit(0));
    let mut cases = vec![];
    let mut s = base.clone();
    s.entry_offset = u32::MAX;
    cases.push(s);
    let mut s = base.clone();
    s.image_base = 1;
    cases.push(s);
    let mut s = base.clone();
    s.imports.clear();
    cases.push(s);
    let mut s = base.clone();
    s.imports[0].name = "bad\0name".into();
    cases.push(s);
    let mut s = base.clone();
    s.code_symbols = vec![(Symbol(0), 0)];
    cases.push(s);
    let mut s = base.clone();
    s.code_symbols = vec![(Symbol(1), u32::MAX)];
    cases.push(s);
    let data = DataSection {
        name: ".rdata".into(),
        bytes: vec![0; 8],
        virtual_size: 8,
        access: Access::ReadOnly,
        symbols: vec![(Symbol(1), 0)],
    };
    for change in 0..7 {
        let mut s = base.clone();
        let mut d = data.clone();
        match change {
            0 => d.name = ".text".into(),
            1 => d.virtual_size = 1,
            2 => d.access = Access::ZeroFill,
            3 => d.symbols[0].1 = 9,
            4 => d.virtual_size = u32::MAX,
            5 => d.name = "toolongsection".into(),
            _ => d.name = "".into(),
        };
        s.data.push(d);
        cases.push(s);
    }
    for fix in [
        PointerFixup {
            section: 9,
            offset: 0,
            target: Symbol(1),
            addend: 0,
        },
        PointerFixup {
            section: 0,
            offset: 1,
            target: Symbol(1),
            addend: 0,
        },
        PointerFixup {
            section: 0,
            offset: 8,
            target: Symbol(1),
            addend: 0,
        },
        PointerFixup {
            section: 0,
            offset: 0,
            target: Symbol(99),
            addend: 0,
        },
        PointerFixup {
            section: 0,
            offset: 0,
            target: Symbol(1),
            addend: i64::MIN,
        },
    ] {
        let mut s = base.clone();
        s.data.push(data.clone());
        s.pointers.push(fix);
        cases.push(s);
    }
    let mut s = base.clone();
    s.data.push(data.clone());
    s.pointers = vec![
        PointerFixup {
            section: 0,
            offset: 0,
            target: Symbol(1),
            addend: 0
        };
        2
    ];
    cases.push(s);
    let mut s = base;
    s.data.push(data);
    s.exception_directory = Some(DirectoryRef {
        section: 0,
        offset: 0,
        size: 12,
    });
    cases.push(s);
    for s in cases {
        assert!(build(&s).is_err(), "accepted invalid spec {s:?}");
    }
}
#[test]
fn truncated_and_mutated_images_do_not_panic() {
    let image = build(&spec(&exit(0))).unwrap();
    for n in 0..image.bytes.len() {
        assert!(validate(&image.bytes[..n]).is_err(), "truncated at {n}");
    }
    for at in [
        0,
        60,
        128,
        132,
        134,
        148,
        152,
        184,
        188,
        208,
        212,
        260,
        392 + 12,
        392 + 20,
    ] {
        let mut b = image.bytes.clone();
        for byte in &mut b[at..at + 4] {
            *byte = 0xff;
        }
        assert!(validate(&b).is_err(), "mutation at {at}");
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
fn run(image: &Image, expected: i32) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ID: AtomicUsize = AtomicUsize::new(0);
    let p = std::env::temp_dir().join(format!(
        "dynamic image {} {}.exe",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&p, &image.bytes).unwrap();
    let mut c = std::process::Command::new(&p).spawn().unwrap();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = c.try_wait().unwrap() {
            break s;
        }
        if start.elapsed().as_secs() >= 5 {
            c.kill().unwrap();
            c.wait().unwrap();
            panic!("dynamic process timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    std::fs::remove_file(p).unwrap();
    assert_eq!(status.code(), Some(expected));
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn windows_runs_large_pic_and_multiple_dll_imports() {
    let mut instructions = vec![Instruction::Nop; 6000];
    instructions.extend([
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Sub,
            reg: Reg::RSP,
            value: 40,
        },
        Instruction::CallImport(Symbol(1)),
        Instruction::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: Reg::RAX,
            src: Reg::RAX,
        },
        Instruction::Set {
            condition: Condition::NotEqual,
            dst: Reg::RCX,
        },
        Instruction::CallImport(Symbol(0)),
    ]);
    let mut s = spec(&instructions);
    s.imports.push(Import {
        symbol: Symbol(1),
        dll: "user32.dll".into(),
        name: "GetDoubleClickTime".into(),
    });
    run(&build(&s).unwrap(), 1);
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn loader_relocates_absolute_pointer_and_zero_fill() {
    let r10 = Reg::new(10).unwrap();
    let r11 = Reg::new(11).unwrap();
    let instructions = vec![
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Sub,
            reg: Reg::RSP,
            value: 40,
        },
        Instruction::LeaSymbol {
            dst: r10,
            symbol: Symbol(2),
        },
        Instruction::Load {
            width: Width::Qword,
            dst: r10,
            address: Address::base(r10, 0),
        },
        Instruction::Load {
            width: Width::Dword,
            dst: Reg::RCX,
            address: Address::base(r10, 0),
        },
        Instruction::LeaSymbol {
            dst: r11,
            symbol: Symbol(3),
        },
        Instruction::Load {
            width: Width::Dword,
            dst: Reg::RAX,
            address: Address::base(r11, 0),
        },
        Instruction::Binary {
            width: Width::Dword,
            op: Binary::Add,
            dst: Reg::RCX,
            src: Reg::RAX,
        },
        Instruction::CallImport(Symbol(0)),
    ];
    let mut s = spec(&instructions);
    let mut bytes = vec![0; 8200];
    bytes[4096..4100].copy_from_slice(&37u32.to_le_bytes());
    s.data = vec![
        DataSection {
            name: ".rdata".into(),
            bytes,
            virtual_size: 8200,
            access: Access::ReadOnly,
            symbols: vec![(Symbol(1), 4096)],
        },
        DataSection {
            name: ".data".into(),
            bytes: vec![0; 8],
            virtual_size: 8,
            access: Access::ReadWrite,
            symbols: vec![(Symbol(2), 0)],
        },
        DataSection {
            name: ".bss".into(),
            bytes: vec![],
            virtual_size: 4096,
            access: Access::ZeroFill,
            symbols: vec![(Symbol(3), 0)],
        },
    ];
    s.pointers.push(PointerFixup {
        section: 1,
        offset: 0,
        target: Symbol(1),
        addend: 0,
    });
    let image = build(&s).unwrap();
    let data = &image.sections[2];
    let value = u64::from_le_bytes(
        image.bytes[data.raw_offset as usize..data.raw_offset as usize + 8]
            .try_into()
            .unwrap(),
    );
    assert_eq!(value, s.image_base + image.symbols[&Symbol(1)] as u64);
    std::fs::write("target/P04-relocation-audit.exe", &image.bytes).unwrap();
    run(&image, 37);
    // Compare the loaded absolute pointer against the preferred-base value. A
    // status of one proves that this process actually used a changed load base.
    let probe = vec![
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Sub,
            reg: Reg::RSP,
            value: 40,
        },
        Instruction::LeaSymbol {
            dst: r10,
            symbol: Symbol(2),
        },
        Instruction::Load {
            width: Width::Qword,
            dst: r10,
            address: Address::base(r10, 0),
        },
        Instruction::MovImm {
            width: Width::Qword,
            dst: Reg::RAX,
            value: 0,
        },
        Instruction::Binary {
            width: Width::Qword,
            op: Binary::Compare,
            dst: r10,
            src: Reg::RAX,
        },
        Instruction::Set {
            condition: Condition::NotEqual,
            dst: Reg::RCX,
        },
        Instruction::CallImport(Symbol(0)),
    ];
    s.text = encode(&probe).unwrap();
    let preliminary = build(&s).unwrap();
    let mut probe = probe;
    probe[3] = Instruction::MovImm {
        width: Width::Qword,
        dst: Reg::RAX,
        value: s.image_base + preliminary.symbols[&Symbol(1)] as u64,
    };
    s.text = encode(&probe).unwrap();
    run(&build(&s).unwrap(), 1);
}
#[test]
fn maximum_section_table_and_invalid_directory_boundaries() {
    let mut s = spec(&exit(0));
    for n in 0..93 {
        s.data.push(DataSection {
            name: format!(".d{n}"),
            bytes: vec![0],
            virtual_size: 1,
            access: Access::ReadOnly,
            symbols: vec![],
        });
    }
    let image = build(&s).unwrap();
    assert_eq!(image.sections.len(), 96);
    assert!(image.entry_rva >= 8192);
    validate(&image.bytes).unwrap();
    s.data.push(DataSection {
        name: ".last".into(),
        bytes: vec![0],
        virtual_size: 1,
        access: Access::ReadOnly,
        symbols: vec![],
    });
    assert!(build(&s).is_err());
    let image = build(&spec(&exit(0))).unwrap();
    for directory in [1usize, 3, 5, 12] {
        for invalid in [u32::MAX, 0, 4095] {
            let mut b = image.bytes.clone();
            if directory == 3 {
                b[152 + 116 + directory * 8..152 + 120 + directory * 8]
                    .copy_from_slice(&12u32.to_le_bytes());
            }
            b[152 + 112 + directory * 8..152 + 116 + directory * 8]
                .copy_from_slice(&invalid.to_le_bytes());
            assert!(validate(&b).is_err());
        }
    }
    let mut seed = 2212u64;
    for _ in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = (seed as usize) % 2048;
        let mut bytes = vec![0; len];
        for b in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 32) as u8;
        }
        assert!(std::panic::catch_unwind(|| validate(&bytes)).is_ok());
        let mut mutated = image.bytes.clone();
        let at = (seed as usize) % (mutated.len() - 4);
        mutated[at..at + 4].copy_from_slice(&(seed as u32).to_le_bytes());
        assert!(std::panic::catch_unwind(|| validate(&mutated)).is_ok());
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn position_independent_image_really_moves_and_large_headers_execute() {
    let r10 = Reg::new(10).unwrap();
    let instructions = vec![
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Sub,
            reg: Reg::RSP,
            value: 40,
        },
        Instruction::LeaSymbol {
            dst: Reg::RAX,
            symbol: Symbol(9),
        },
        Instruction::MovImm {
            width: Width::Qword,
            dst: r10,
            value: 0,
        },
        Instruction::Binary {
            width: Width::Qword,
            op: Binary::Compare,
            dst: Reg::RAX,
            src: r10,
        },
        Instruction::Set {
            condition: Condition::NotEqual,
            dst: Reg::RCX,
        },
        Instruction::CallImport(Symbol(0)),
    ];
    let mut s = spec(&instructions);
    s.code_symbols.push((Symbol(9), 0));
    let preliminary = build(&s).unwrap();
    let mut instructions = instructions;
    instructions[2] = Instruction::MovImm {
        width: Width::Qword,
        dst: r10,
        value: s.image_base + preliminary.symbols[&Symbol(9)] as u64,
    };
    s.text = encode(&instructions).unwrap();
    run(&build(&s).unwrap(), 1);
    let mut s = spec(&exit(91));
    for n in 0..93 {
        s.data.push(DataSection {
            name: format!(".d{n}"),
            bytes: vec![0],
            virtual_size: 1,
            access: Access::ReadOnly,
            symbols: vec![],
        });
    }
    run(&build(&s).unwrap(), 91);
}
