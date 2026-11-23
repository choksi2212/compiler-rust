use compiler::ir::{Ir, Value};
use compiler::native_runtime::{RuntimeOptions, image};
#[test]
fn runtime_configuration_and_ir_errors() {
    assert!(image(&[Ir::Halt], &RuntimeOptions { output_capacity: 0 }).is_err());
    assert!(
        image(
            &[Ir::Halt],
            &RuntimeOptions {
                output_capacity: u32::MAX
            }
        )
        .is_err()
    );
    assert!(image(&[Ir::Copy(0, 99), Ir::Halt], &RuntimeOptions::default()).is_err());
    assert!(
        image(
            &[Ir::Const(0, Value::Str("embedded\0nul".into())), Ir::Halt],
            &RuntimeOptions::default()
        )
        .is_err()
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows {
    use super::*;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    pub(super) fn execute(ir: &[Ir], capacity: u32) -> (i32, Vec<u8>, Vec<u8>) {
        let output = image(
            ir,
            &RuntimeOptions {
                output_capacity: capacity,
            },
        )
        .unwrap();
        execute_image(&output)
    }
    pub(super) fn execute_image(output: &compiler::pe::Image) -> (i32, Vec<u8>, Vec<u8>) {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "owned native runtime {} {}.exe",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, &output.bytes).unwrap();
        let mut child = Command::new(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out = std::thread::spawn(move || {
            let mut b = vec![];
            stdout.read_to_end(&mut b).unwrap();
            b
        });
        let err = std::thread::spawn(move || {
            let mut b = vec![];
            stderr.read_to_end(&mut b).unwrap();
            b
        });
        let start = std::time::Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed().as_secs() >= 5 {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("runtime program timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let stdout = out.join().unwrap();
        let stderr = err.join().unwrap();
        std::fs::remove_file(path).unwrap();
        (status.code().unwrap(), stdout, stderr)
    }
    #[test]
    fn integer_string_and_return_count() {
        let ir = vec![
            Ir::Const(0, Value::Int(i32::MIN)),
            Ir::Const(1, Value::Int(i32::MAX)),
            Ir::Const(2, Value::Str("Hello\t世界".into())),
            Ir::Unary(3, "strlen".into(), 2),
            Ir::Print("%d %d %s %d\n".into(), vec![0, 1, 2, 3], 4),
            Ir::Print("count=%d\n".into(), vec![4], 5),
            Ir::Halt,
        ];
        let first = "-2147483648 2147483647 Hello\t世界 12\n";
        let expected = format!("{first}count={}\n", first.len());
        let audit = image(
            &ir,
            &RuntimeOptions {
                output_capacity: 4096,
            },
        )
        .unwrap();
        std::fs::write("target/P07-runtime-audit.exe", &audit.bytes).unwrap();
        let (status, out, err) = execute_image(&audit);
        assert_eq!(status, 0);
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        assert_eq!(out, expected.as_bytes());
    }
    #[test]
    fn empty_zero_negative_and_exact_capacity() {
        for (value, expected) in [
            (0, "0"),
            (-1, "-1"),
            (1, "1"),
            (1234567890, "1234567890"),
            (i32::MIN, "-2147483648"),
        ] {
            let ir = [
                Ir::Const(0, Value::Int(value)),
                Ir::Print("%d".into(), vec![0], 1),
                Ir::Halt,
            ];
            let (status, out, err) = execute(&ir, expected.len() as u32);
            assert_eq!(
                (status, out, err),
                (0, expected.as_bytes().to_vec(), vec![])
            );
            let (status, out, err) = execute(&ir, 1);
            if expected.len() > 1 {
                assert_eq!(status, 4);
                assert!(out.is_empty());
                assert!(String::from_utf8_lossy(&err).contains("capacity"));
            }
        }
        let (status, out, err) = execute(
            &[
                Ir::Const(0, Value::Str("".into())),
                Ir::Print("%s".into(), vec![0], 1),
                Ir::Halt,
            ],
            1,
        );
        assert_eq!((status, out, err), (0, vec![], vec![]));
    }
    #[test]
    fn runtime_errors_discard_buffered_output() {
        for (fault, status, message) in [
            (
                vec![
                    Ir::Const(2, Value::Int(1)),
                    Ir::Const(3, Value::Int(0)),
                    Ir::Binary(4, "/".into(), 2, 3),
                ],
                1,
                "division by zero",
            ),
            (
                vec![
                    Ir::Const(2, Value::Int(i32::MAX)),
                    Ir::Const(3, Value::Int(1)),
                    Ir::Binary(4, "+".into(), 2, 3),
                ],
                2,
                "overflow",
            ),
            (vec![Ir::Reset(2), Ir::Copy(3, 2)], 3, "uninitialized"),
        ] {
            let mut ir = vec![Ir::Print("must be discarded".into(), vec![], 0)];
            ir.extend(fault);
            ir.push(Ir::Halt);
            let (actual, out, err) = execute(&ir, 1024);
            assert_eq!(actual, status);
            assert!(out.is_empty());
            assert!(String::from_utf8_lossy(&err).contains(message));
        }
    }
    #[test]
    fn large_output_and_many_arguments() {
        let text = "a".repeat(100000);
        let ir = [
            Ir::Const(0, Value::Str(text.clone())),
            Ir::Print("%s%s".into(), vec![0, 0], 1),
            Ir::Halt,
        ];
        let (status, out, err) = execute(&ir, 300000);
        assert_eq!(status, 0);
        assert!(err.is_empty());
        assert_eq!(out, [text.as_bytes(), text.as_bytes()].concat());
        let mut ir: Vec<_> = (0..12)
            .map(|id| Ir::Const(id, Value::Int(id as i32)))
            .collect();
        ir.push(Ir::Print(
            "%d %d %d %d %d %d %d %d %d %d %d %d".into(),
            (0..12).collect(),
            12,
        ));
        ir.push(Ir::Halt);
        let (status, out, err) = execute(&ir, 1000);
        assert_eq!(
            (status, out, err),
            (0, b"0 1 2 3 4 5 6 7 8 9 10 11".to_vec(), vec![])
        );
    }
}
#[test]
fn raw_nul_source_is_rejected_before_native_string_lowering() {
    assert!(compiler::lexer::lex("\"a\0b\"").is_err());
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn actual_os_failure_paths_and_literal_reassignment() {
    use windows::execute_image;
    // Invalid VirtualAlloc flags and invalid standard-handle selectors are real
    // bad OS inputs. The imported APIs execute normally; no results are mocked.
    let ir = [Ir::Print("buffered".into(), vec![], 0), Ir::Halt];
    for (pattern, patch, status, message) in [
        (
            vec![0x41, 0xb8, 0, 0x30, 0, 0],
            vec![0x41, 0xb8, 0, 0, 0, 0],
            6,
            "memory service",
        ),
        (
            vec![0xb9, 0xf5, 0xff, 0xff, 0xff],
            vec![0xb9, 0, 0, 0, 0],
            7,
            "IO failure",
        ),
    ] {
        let mut native = image(&ir, &RuntimeOptions::default()).unwrap();
        let section = &native.sections[0];
        let start = section.raw_offset as usize;
        let end = start + section.virtual_size as usize;
        let at = native.bytes[start..end]
            .windows(pattern.len())
            .position(|v| v == pattern)
            .unwrap()
            + start;
        native.bytes[at..at + pattern.len()].copy_from_slice(&patch);
        let (actual, out, err) = execute_image(&native);
        assert_eq!(actual, status);
        assert!(out.is_empty());
        assert!(String::from_utf8_lossy(&err).contains(message));
    }
    let ir = [
        Ir::Const(0, Value::Str("a".into())),
        Ir::Copy(1, 0),
        Ir::Print("%s".into(), vec![1], 2),
        Ir::Const(0, Value::Str("b".into())),
        Ir::Print("%s".into(), vec![0], 3),
        Ir::Halt,
    ];
    let (status, out, err) = windows::execute(&ir, 10);
    assert_eq!((status, out, err), (0, b"ab".to_vec(), vec![]));
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn formatter_integer_matrix_in_one_real_program() {
    let mut values: Vec<i32> = (-128..=128).collect();
    values.extend([
        i32::MIN,
        i32::MIN + 1,
        -1000000000,
        -46341,
        46341,
        1000000000,
        i32::MAX - 1,
        i32::MAX,
    ]);
    let mut ir = vec![];
    let mut expected = String::new();
    for (index, value) in values.iter().enumerate() {
        ir.push(Ir::Const(index * 2, Value::Int(*value)));
        ir.push(Ir::Print("%d,".into(), vec![index * 2], index * 2 + 1));
        expected.push_str(&format!("{value},"));
    }
    ir.push(Ir::Halt);
    let (status, out, err) = windows::execute(&ir, 10000);
    assert_eq!((status, out, err), (0, expected.into_bytes(), vec![]));
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn formatter_checks_actual_argument_arity_and_format() {
    use compiler::abi::*;
    use compiler::pe::*;
    use compiler::x64::*;
    for (format, count, expected) in [
        ("%q", 1, 5),
        ("%", 1, 5),
        ("%d", 0, 5),
        ("%d", 2, 5),
        ("", 1, 5),
        ("literal", 0, 0),
    ] {
        let mut labels = LabelAllocator::default();
        let entry = labels.fresh().unwrap();
        let helper = compiler::native_runtime::formatter(&mut labels).unwrap();
        let frame = FramePlan::new(&FrameRequest {
            has_calls: true,
            ..FrameRequest::default()
        })
        .unwrap();
        let prologue = frame.prologue(labels.fresh().unwrap()).unwrap();
        let mut code = prologue.instructions;
        code.extend([
            Instruction::LeaSymbol {
                dst: Reg::RCX,
                symbol: Symbol(20),
            },
            Instruction::LeaSymbol {
                dst: Reg::RDX,
                symbol: Symbol(22),
            },
        ]);
        if count > 0 {
            code.push(Instruction::LeaSymbol {
                dst: Reg::new(8).unwrap(),
                symbol: Symbol(23),
            });
        } else {
            code.push(Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::new(8).unwrap(),
                value: 0,
            });
        }
        code.extend([
            Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::new(9).unwrap(),
                value: count,
            },
            Instruction::Call(entry),
            Instruction::LeaSymbol {
                dst: Reg::new(10).unwrap(),
                symbol: Symbol(20),
            },
            Instruction::Load {
                width: Width::Dword,
                dst: Reg::RCX,
                address: Address::base(Reg::new(10).unwrap(), 24),
            },
            Instruction::CallImport(Symbol(0)),
            Instruction::Bind(entry),
        ]);
        code.extend(helper.instructions);
        let text = encode(&code).unwrap();
        let split = text
            .listing
            .iter()
            .find(|r| matches!(r.instruction,Instruction::Bind(l)if l==entry))
            .unwrap()
            .offset as u32;
        let end = text.bytes.len() as u32;
        let mut state = vec![0; 96];
        state[16..24].copy_from_slice(&64u64.to_le_bytes());
        let mut format_bytes = format.as_bytes().to_vec();
        format_bytes.push(0);
        let mut data = vec![
            DataSection {
                name: ".rdata".into(),
                virtual_size: format_bytes.len() as u32,
                bytes: format_bytes,
                access: Access::ReadOnly,
                symbols: vec![(Symbol(22), 0)],
            },
            DataSection {
                name: ".data".into(),
                virtual_size: 96,
                bytes: state,
                access: Access::ReadWrite,
                symbols: vec![(Symbol(20), 0), (Symbol(21), 32)],
            },
        ];
        if count > 0 {
            data.push(DataSection {
                name: ".args".into(),
                bytes: vec![0; count as usize * 8],
                virtual_size: count as u32 * 8,
                access: Access::ReadOnly,
                symbols: vec![(Symbol(23), 0)],
            });
        }
        let spec = ImageSpec {
            text,
            code_symbols: vec![],
            data,
            imports: vec![Import {
                symbol: Symbol(0),
                dll: "kernel32.dll".into(),
                name: "ExitProcess".into(),
            }],
            pointers: vec![PointerFixup {
                section: 1,
                offset: 0,
                target: Symbol(21),
                addend: 0,
            }],
            entry_offset: 0,
            image_base: 0x140000000,
            exception_directory: None,
        };
        let spec = with_unwind(
            &spec,
            &[
                FunctionUnwind {
                    begin_offset: 0,
                    end_offset: split,
                    unwind: prologue.unwind,
                },
                FunctionUnwind {
                    begin_offset: split,
                    end_offset: end,
                    unwind: helper.unwind,
                },
            ],
        )
        .unwrap();
        let result = windows::execute_image(&build(&spec).unwrap());
        assert_eq!(result, (expected, vec![], vec![]), "{format} count{count}");
    }
}
