use compiler::{
    ir::{Ir, Value},
    native_integer::{Status, lower_integer},
    x64::*,
};
#[test]
fn unsupported_and_malformed_ir_is_not_a_fallback() {
    for code in [
        vec![Ir::Print("text".into(), vec![], 0), Ir::Halt],
        vec![Ir::Const(0, Value::Str("s".into())), Ir::Halt],
        vec![Ir::Copy(0, 99), Ir::Halt],
    ] {
        assert!(lower_integer(&code, &mut LabelAllocator::default()).is_err());
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows {
    use super::*;
    use compiler::abi::RuntimeFunction;
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(
            address: *mut c_void,
            size: usize,
            allocation: u32,
            protect: u32,
        ) -> *mut c_void;
        fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        fn RtlAddFunctionTable(table: *const RuntimeFunction, count: u32, base: u64) -> u8;
        fn RtlDeleteFunctionTable(table: *const RuntimeFunction) -> u8;
    }
    pub(super) fn native(ir: &[Ir]) -> i32 {
        let f = lower_integer(ir, &mut LabelAllocator::default()).unwrap();
        let encoded = encode(&f.instructions).unwrap();
        assert!(encoded.external_fixups.is_empty());
        let code_pages = encoded.bytes.len().div_ceil(4096) * 4096;
        let table = RuntimeFunction {
            begin: 0,
            end: encoded.bytes.len() as u32,
            unwind: code_pages as u32,
        };
        // SAFETY: owned bounded allocation remains live/registered throughout the
        // Windows-ABI native call. Only code pages become executable, never writable.
        unsafe {
            let address = VirtualAlloc(std::ptr::null_mut(), code_pages + 4096, 0x3000, 4);
            assert!(!address.is_null());
            std::ptr::copy_nonoverlapping(
                encoded.bytes.as_ptr(),
                address.cast::<u8>(),
                encoded.bytes.len(),
            );
            std::ptr::copy_nonoverlapping(
                f.unwind.as_ptr(),
                address.cast::<u8>().add(code_pages),
                f.unwind.len(),
            );
            let mut old = 0;
            assert_ne!(VirtualProtect(address, code_pages, 0x20, &mut old), 0);
            assert_ne!(RtlAddFunctionTable(&table, 1, address as u64), 0);
            let call: unsafe extern "system" fn() -> i32 = std::mem::transmute(address);
            let status = call();
            assert_ne!(RtlDeleteFunctionTable(&table), 0);
            assert_ne!(VirtualFree(address, 0, 0x8000), 0);
            status
        }
    }
    pub(super) fn checked_result(a: i32, b: i32, op: &str, expected: i32) -> Vec<Ir> {
        vec![
            Ir::Const(0, Value::Int(a)),
            Ir::Const(1, Value::Int(b)),
            Ir::Binary(2, op.into(), 0, 1),
            Ir::Const(3, Value::Int(expected)),
            Ir::Binary(4, "==".into(), 2, 3),
            Ir::Zero(4, 0),
            Ir::Halt,
            Ir::Label(0),
            Ir::Const(5, Value::Int(1)),
            Ir::Const(6, Value::Int(0)),
            Ir::Binary(7, "/".into(), 5, 6),
            Ir::Halt,
        ]
    }
    #[test]
    fn arithmetic_and_comparison_matrix_executes_on_cpu() {
        for a in -12i32..=12 {
            for b in -12i32..=12 {
                for op in ["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!="] {
                    if b == 0 && matches!(op, "/" | "%") {
                        continue;
                    }
                    let expected = match op {
                        "+" => a + b,
                        "-" => a - b,
                        "*" => a * b,
                        "/" => a / b,
                        "%" => a % b,
                        "<" => (a < b) as i32,
                        "<=" => (a <= b) as i32,
                        ">" => (a > b) as i32,
                        ">=" => (a >= b) as i32,
                        "==" => (a == b) as i32,
                        _ => (a != b) as i32,
                    };
                    assert_eq!(
                        native(&checked_result(a, b, op, expected)),
                        0,
                        "{a}{op}{b} expected {expected}"
                    );
                }
            }
        }
    }
    #[test]
    fn checked_errors_and_sparse_slots() {
        for (a, b, op, status) in [
            (1, 0, "/", Status::DivisionByZero),
            (1, 0, "%", Status::DivisionByZero),
            (i32::MAX, 1, "+", Status::Overflow),
            (i32::MIN, 1, "-", Status::Overflow),
            (i32::MAX, 2, "*", Status::Overflow),
            (i32::MIN, -1, "/", Status::Overflow),
            (i32::MIN, -1, "%", Status::Overflow),
        ] {
            assert_eq!(
                native(&[
                    Ir::Const(0, Value::Int(a)),
                    Ir::Const(1, Value::Int(b)),
                    Ir::Binary(2, op.into(), 0, 1),
                    Ir::Halt
                ]),
                status as i32
            );
        }
        assert_eq!(
            native(&[
                Ir::Const(0, Value::Int(i32::MIN)),
                Ir::Unary(1, "-".into(), 0),
                Ir::Halt
            ]),
            Status::Overflow as i32
        );
        assert_eq!(
            native(&[Ir::Reset(0), Ir::Copy(1, 0), Ir::Halt]),
            Status::Uninitialized as i32
        );
        assert_eq!(
            native(&[
                Ir::Const(0, Value::Int(7)),
                Ir::Reset(0),
                Ir::Copy(1, 0),
                Ir::Halt
            ]),
            Status::Uninitialized as i32
        );
        assert_eq!(
            native(&[
                Ir::Const(usize::MAX, Value::Int(7)),
                Ir::Copy(0, usize::MAX),
                Ir::Halt
            ]),
            0
        );
    }
    fn compile(source: &str, optimize: bool) -> Vec<Ir> {
        let ast = compiler::parser::parse(compiler::lexer::lex(source).unwrap()).unwrap();
        compiler::semantic::check(&ast).unwrap();
        let ir = compiler::ir::lower(&ast);
        if optimize {
            compiler::optimize::optimize(&ir)
        } else {
            ir
        }
    }
    #[test]
    fn real_source_loops_scopes_and_short_circuit() {
        let sources = [
            (
                "int main(){int sum=0;for(int i=0;i<10;i=i+1){sum=sum+i;}if(sum!=45){int error=1/0;}return 0;}",
                0,
            ),
            (
                "int main(){int x=4;{int x=8;if(x!=8){int e=1/0;}}if(x!=4){int e=1/0;}return 0;}",
                0,
            ),
            (
                "int main(){int x=0&&(1/0);int y=1||(1/0);if(x!=0||y!=1){int e=1/0;}return 0;}",
                0,
            ),
            (
                "int main(){for(int i=0;i<2;i=i+1){int x;if(i==0){x=9;}else{int y=x;}}return 0;}",
                3,
            ),
            ("int main(){int x=1;{int x=x;}return 0;}", 3),
        ];
        for (s, status) in sources {
            for opt in [false, true] {
                assert_eq!(native(&compile(s, opt)), status, "{s} opt{opt}");
            }
        }
    }
    #[test]
    fn large_frame_probes_and_label_namespaces() {
        let mut body = String::from("int main(){");
        for i in 0..600 {
            body.push_str(&format!("int x{i}={i};"));
        }
        body.push_str("if(x599!=599){int e=1/0;}return 0;}");
        let ir = compile(&body, false);
        let mut labels = LabelAllocator::default();
        let reserved = labels.fresh().unwrap();
        let f = lower_integer(&ir, &mut labels).unwrap();
        assert!(f.frame.allocation() > 4096);
        assert!(
            !f.instructions
                .iter()
                .any(|i| matches!(i,Instruction::Bind(l)if *l==reserved))
        );
        assert_eq!(native(&ir), 0);
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn integer_boundaries_and_all_checked_faults_on_cpu() {
    use windows::*;
    let values = [
        i32::MIN,
        i32::MIN + 1,
        -65536,
        -46341,
        -46340,
        -1,
        0,
        1,
        46340,
        46341,
        65536,
        i32::MAX - 1,
        i32::MAX,
    ];
    for a in values {
        for b in values {
            for op in ["+", "-", "*", "/", "%"] {
                let result = match op {
                    "+" => a.checked_add(b),
                    "-" => a.checked_sub(b),
                    "*" => a.checked_mul(b),
                    "/" => a.checked_div(b),
                    _ => a.checked_rem(b),
                };
                if let Some(expected) = result {
                    assert_eq!(native(&checked_result(a, b, op, expected)), 0, "{a}{op}{b}");
                } else {
                    let expected = if b == 0 && matches!(op, "/" | "%") {
                        Status::DivisionByZero
                    } else {
                        Status::Overflow
                    };
                    assert_eq!(
                        native(&[
                            Ir::Const(0, Value::Int(a)),
                            Ir::Const(1, Value::Int(b)),
                            Ir::Binary(2, op.into(), 0, 1),
                            Ir::Halt
                        ]),
                        expected as i32,
                        "{a}{op}{b}"
                    );
                }
            }
        }
    }
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn source_integer_programs_execute_as_standalone_pe() {
    use compiler::abi::*;
    use compiler::pe::*;
    for (expected, status) in [(720, 0), (721, 1)] {
        let source = format!(
            "int main(){{int n=6;int result=1;while(n>1){{result=result*n;n=n-1;}}if(result!={expected}){{int error=1/0;}}return 0;}}"
        );
        let ast = compiler::parser::parse(compiler::lexer::lex(&source).unwrap()).unwrap();
        compiler::semantic::check(&ast).unwrap();
        let ir = compiler::ir::lower(&ast);
        let mut labels = LabelAllocator::default();
        let root = FramePlan::new(&FrameRequest {
            has_calls: true,
            ..FrameRequest::default()
        })
        .unwrap();
        let root_prologue = root.prologue(labels.fresh().unwrap()).unwrap();
        let entry = labels.fresh().unwrap();
        let body = lower_integer(&ir, &mut labels).unwrap();
        let mut code = root_prologue.instructions;
        code.extend([
            Instruction::Call(entry),
            Instruction::Mov {
                width: Width::Dword,
                dst: Reg::RCX,
                src: Reg::RAX,
            },
            Instruction::CallImport(Symbol(0)),
            Instruction::Bind(entry),
        ]);
        code.extend(body.instructions);
        let fragment = encode(&code).unwrap();
        let split = fragment
            .listing
            .iter()
            .find(|r| matches!(r.instruction,Instruction::Bind(l)if l==entry))
            .unwrap()
            .offset as u32;
        let end = fragment.bytes.len() as u32;
        let spec = ImageSpec {
            text: fragment,
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
        };
        let spec = with_unwind(
            &spec,
            &[
                FunctionUnwind {
                    begin_offset: 0,
                    end_offset: split,
                    unwind: root_prologue.unwind,
                },
                FunctionUnwind {
                    begin_offset: split,
                    end_offset: end,
                    unwind: body.unwind,
                },
            ],
        )
        .unwrap();
        let image = build(&spec).unwrap();
        let p = std::env::temp_dir().join(format!(
            "native integer source {} {expected}.exe",
            std::process::id()
        ));
        std::fs::write(&p, &image.bytes).unwrap();
        let mut child = std::process::Command::new(&p).spawn().unwrap();
        let start = std::time::Instant::now();
        let result = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed().as_secs() >= 5 {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("integer program timeout");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        std::fs::remove_file(p).unwrap();
        assert_eq!(result.code(), Some(status));
    }
}
#[test]
fn slot_budget_is_explicit() {
    let mut ir: Vec<Ir> = (0..65536).map(|id| Ir::Const(id, Value::Int(1))).collect();
    ir.push(Ir::Halt);
    assert!(
        lower_integer(&ir, &mut LabelAllocator::default())
            .unwrap_err()
            .contains("slot budget")
    );
}
