use compiler::abi::*;
use compiler::pe::*;
use compiler::x64::*;
#[test]
fn rejects_invalid_unwind_records() {
    for bytes in [
        vec![],
        vec![1, 1, 1, 0],
        vec![2, 0, 0, 0],
        vec![1, 1, 0, 0, 0, 0, 0, 0],
        vec![1, 1, 1, 0, 1, 0x40, 0, 0],
        vec![1, 1, 1, 0, 1, 0x11, 0, 0],
        vec![1, 1, 1, 0, 0, 0x02, 0, 0],
    ] {
        assert!(validate_unwind(&bytes).is_err(), "{bytes:?}");
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn windows_finds_serialized_function_unwind_table() {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        locals: vec![(8, 8)],
        ..FrameRequest::default()
    })
    .unwrap();
    let prologue = frame.prologue(Label(100)).unwrap();
    let mut instructions = prologue.instructions;
    instructions.extend([
        Instruction::LeaSymbol {
            dst: Reg::RCX,
            symbol: Symbol(10),
        },
        Instruction::Lea {
            dst: Reg::RDX,
            address: Address::base(Reg::RSP, frame.local_offsets()[0] as i32),
        },
        Instruction::MovImm {
            width: Width::Qword,
            dst: Reg::new(8).unwrap(),
            value: 0,
        },
        Instruction::CallImport(Symbol(1)),
        Instruction::Binary {
            width: Width::Qword,
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
    let text = encode(&instructions).unwrap();
    let length = text.bytes.len() as u32;
    let spec = ImageSpec {
        text,
        code_symbols: vec![(Symbol(10), 0)],
        data: vec![],
        imports: vec![
            Import {
                symbol: Symbol(0),
                dll: "kernel32.dll".into(),
                name: "ExitProcess".into(),
            },
            Import {
                symbol: Symbol(1),
                dll: "kernel32.dll".into(),
                name: "RtlLookupFunctionEntry".into(),
            },
        ],
        pointers: vec![],
        entry_offset: 0,
        image_base: 0x140000000,
        exception_directory: None,
    };
    let spec = with_unwind(
        &spec,
        &[FunctionUnwind {
            begin_offset: 0,
            end_offset: length,
            unwind: prologue.unwind,
        }],
    )
    .unwrap();
    let image = build(&spec).unwrap();
    validate(&image.bytes).unwrap();
    std::fs::write("target/P05-unwind-audit.exe", &image.bytes).unwrap();
    let path = std::env::temp_dir().join(format!(
        "native unwind directory {}.exe",
        std::process::id()
    ));
    std::fs::write(&path, &image.bytes).unwrap();
    let mut child = std::process::Command::new(&path).spawn().unwrap();
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if start.elapsed().as_secs() >= 5 {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("unwind image timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    std::fs::remove_file(path).unwrap();
    assert_eq!(status.code(), Some(1));
}
#[test]
fn metadata_attachment_checks_ranges_transactionally() {
    let spec = ImageSpec {
        text: encode(&[Instruction::Ret]).unwrap(),
        code_symbols: vec![],
        data: vec![],
        imports: vec![],
        pointers: vec![],
        entry_offset: 0,
        image_base: 0x140000000,
        exception_directory: None,
    };
    let valid = FunctionUnwind {
        begin_offset: 0,
        end_offset: 1,
        unwind: vec![1, 0, 0, 0],
    };
    let attached = with_unwind(&spec, std::slice::from_ref(&valid)).unwrap();
    assert!(spec.data.is_empty());
    assert_eq!(attached.data.len(), 2);
    assert!(attached.exception_directory.is_some());
    assert!(with_unwind(&spec, &[]).is_err());
    assert!(with_unwind(&attached, std::slice::from_ref(&valid)).is_err());
    for f in [
        FunctionUnwind {
            begin_offset: 1,
            end_offset: 1,
            unwind: vec![1, 0, 0, 0],
        },
        FunctionUnwind {
            begin_offset: 0,
            end_offset: 2,
            unwind: vec![1, 0, 0, 0],
        },
        FunctionUnwind {
            begin_offset: 0,
            end_offset: 1,
            unwind: vec![1, 2, 0, 0],
        },
        FunctionUnwind {
            begin_offset: 0,
            end_offset: 1,
            unwind: vec![0, 0, 0, 0],
        },
    ] {
        assert!(with_unwind(&spec, &[f]).is_err());
    }
    assert!(with_unwind(&spec, &[valid.clone(), valid]).is_err());
    let mut seed = 2212u64;
    for _ in 0..10000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = vec![0; seed as usize % 520];
        for b in &mut bytes {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (seed >> 32) as u8;
        }
        assert!(std::panic::catch_unwind(|| validate_unwind(&bytes)).is_ok());
    }
}
