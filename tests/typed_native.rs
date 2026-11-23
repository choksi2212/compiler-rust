#![cfg(all(windows, target_arch = "x86_64"))]
use compiler::{
    abi::RuntimeFunction,
    integer::{Binary, Unary},
    typed_ir::*,
    typed_native,
    types::{IntegerKind::*, *},
    x64::{LabelAllocator, encode},
};
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
    fn FlushInstructionCache(process: *mut c_void, address: *const c_void, size: usize) -> i32;
    fn RtlAddFunctionTable(table: *const RuntimeFunction, count: u32, base: u64) -> u8;
    fn RtlDeleteFunctionTable(table: *const RuntimeFunction) -> u8;
}
struct Native {
    address: *mut c_void,
    table: Box<RuntimeFunction>,
}
impl Native {
    fn new(module: &Module, runtime: bool) -> Self {
        let function =
            typed_native::lower(module, &mut LabelAllocator::default(), runtime).unwrap();
        let fragment = encode(&function.instructions).unwrap();
        assert!(fragment.external_fixups.is_empty());
        let pages = fragment.bytes.len().div_ceil(4096) * 4096;
        let table = Box::new(RuntimeFunction {
            begin: 0,
            end: fragment.bytes.len() as u32,
            unwind: pages as u32,
        });
        assert!(function.unwind.len() < 4096);
        // SAFETY: allocation and unwind record remain live through all calls. Code
        // becomes RX before execution; metadata is kept on a separate RW page.
        unsafe {
            let address = VirtualAlloc(std::ptr::null_mut(), pages + 4096, 0x3000, 4);
            assert!(!address.is_null());
            std::ptr::copy_nonoverlapping(
                fragment.bytes.as_ptr(),
                address.cast::<u8>(),
                fragment.bytes.len(),
            );
            std::ptr::copy_nonoverlapping(
                function.unwind.as_ptr(),
                address.cast::<u8>().add(pages),
                function.unwind.len(),
            );
            let mut old = 0;
            assert_ne!(VirtualProtect(address, pages, 0x20, &mut old), 0);
            assert_ne!(
                FlushInstructionCache((-1isize) as *mut c_void, address, fragment.bytes.len()),
                0
            );
            assert_ne!(RtlAddFunctionTable(&*table, 1, address as u64), 0);
            Self { address, table }
        }
    }
    fn call(&self) -> u64 {
        // SAFETY: caller supplies a verified zero-parameter integer signature.
        unsafe {
            let function: unsafe extern "system" fn() -> u64 = std::mem::transmute(self.address);
            function()
        }
    }
    fn context(&self) -> (u64, i32) {
        let mut context = [0u8; 32];
        // SAFETY: runtime-body contract takes the live 32-byte context as RCX.
        let status = unsafe {
            let function: unsafe extern "system" fn(*mut u8) -> u64 =
                std::mem::transmute(self.address);
            function(context.as_mut_ptr())
        };
        (
            status,
            i32::from_le_bytes(context[28..32].try_into().unwrap()),
        )
    }
}
impl Drop for Native {
    fn drop(&mut self) {
        unsafe {
            assert_ne!(RtlDeleteFunctionTable(&*self.table), 0);
            assert_ne!(VirtualFree(self.address, 0, 0x8000), 0);
        }
    }
}
fn program(kind: IntegerKind, op: Binary, a: u64, b: u64) -> Module {
    let mut types = TypeContext::new();
    let ty = types.integer(kind);
    let result = if op.comparison() {
        types.integer(Int)
    } else {
        ty
    };
    let signature = types.function(result, vec![], false, true).unwrap();
    Module {
        externs: vec![],
        data: vec![],
        entry: None,
        types,
        globals: vec![],
        functions: vec![Function {
            name: "fixture".into(),
            linkage: compiler::types::Linkage::Internal,
            id: FunctionId(0),
            symbol: SymbolId(0),
            signature,
            parameters: vec![],
            locals: vec![],
            values: vec![ty, ty, result],
            entry: BlockId(0),
            blocks: vec![Block {
                id: BlockId(0),
                instructions: vec![
                    Instruction::Constant {
                        dst: ValueId(0),
                        bits: a,
                    },
                    Instruction::Constant {
                        dst: ValueId(1),
                        bits: b,
                    },
                    Instruction::Binary {
                        dst: ValueId(2),
                        op,
                        lhs: ValueId(0),
                        rhs: ValueId(1),
                    },
                ],
                end: Terminator::Return(Some(ValueId(2))),
            }],
        }],
    }
}
fn call(module: &Module) -> u64 {
    Native::new(module, false).call()
}
#[test]
fn signed_unsigned_arithmetic_and_comparisons_execute_at_full_width() {
    for kind in [Int, Long, LongLong] {
        for a in -8i64..=8 {
            for b in -8i64..=8 {
                let abits = (a as u64) & mask(kind.bits());
                let bbits = (b as u64) & mask(kind.bits());
                for (op, expected) in [
                    (Binary::Add, a + b),
                    (Binary::Subtract, a - b),
                    (Binary::Multiply, a * b),
                    (Binary::Less, i64::from(a < b)),
                    (Binary::Equal, i64::from(a == b)),
                    (Binary::GreaterEqual, i64::from(a >= b)),
                ] {
                    assert_eq!(
                        call(&program(kind, op, abits, bbits)),
                        expected as u64,
                        "{kind:?} {a} {op:?} {b}"
                    );
                }
            }
        }
    }
    for kind in [UnsignedInt, UnsignedLong, UnsignedLongLong] {
        let maximum = kind.maximum();
        for (op, a, b, expected) in [
            (Binary::Add, maximum, 1, 0),
            (Binary::Subtract, 0, 1, maximum),
            (Binary::Multiply, maximum, maximum, 1),
            (Binary::Divide, maximum, 2, maximum / 2),
            (Binary::Remainder, maximum, 2, 1),
            (Binary::Less, maximum, 1, 0),
            (Binary::Greater, maximum, 1, 1),
            (Binary::Equal, maximum, maximum, 1),
            (Binary::And, maximum, 0x7fffffff, 0x7fffffff),
            (Binary::Xor, maximum, maximum, 0),
        ] {
            assert_eq!(
                call(&program(kind, op, a, b)),
                expected,
                "{kind:?} {a} {op:?} {b}"
            );
        }
    }
}
#[test]
fn casts_bool_and_signed_narrowing_are_actual_cpu_operations() {
    for source in IntegerKind::ALL {
        for destination in IntegerKind::ALL {
            for input in [
                0,
                1,
                127,
                128,
                255,
                256,
                32767,
                32768,
                u32::MAX as u64,
                1u64 << 32,
                1u64 << 63,
                u64::MAX,
            ] {
                let mut m = program(Int, Binary::Add, 0, 0);
                let source_ty = m.types.integer(source);
                let dest_ty = m.types.integer(destination);
                let signature = m.types.function(dest_ty, vec![], false, true).unwrap();
                let f = &mut m.functions[0];
                f.signature = signature;
                f.values = vec![source_ty, dest_ty];
                let bits = if source == Bool {
                    u64::from(input != 0)
                } else {
                    input & mask(source.bits())
                };
                f.blocks[0].instructions = vec![
                    Instruction::Constant {
                        dst: ValueId(0),
                        bits,
                    },
                    Instruction::Convert {
                        dst: ValueId(1),
                        src: ValueId(0),
                    },
                ];
                f.blocks[0].end = Terminator::Return(Some(ValueId(1)));
                // Independent target bit arithmetic, including sign extension before widening.
                let signed = if source.signed() && bits & (1u64 << (source.bits() - 1)) != 0 {
                    i128::from(bits) - (1i128 << source.bits())
                } else {
                    i128::from(bits)
                };
                let raw = if destination == Bool {
                    u64::from(bits != 0)
                } else {
                    signed as u64 & mask(destination.bits())
                };
                let expected =
                    if destination.signed() && raw & (1u64 << (destination.bits() - 1)) != 0 {
                        (i128::from(raw) - (1i128 << destination.bits())) as u64
                    } else {
                        raw
                    };
                assert_eq!(call(&m), expected, "{source:?} {destination:?} {input}");
            }
        }
    }
}
#[test]
fn checked_faults_and_main_result_use_different_channels() {
    for (kind, op, a, b, status) in [
        (Int, Binary::Add, i32::MAX as u64, 1, 2),
        (LongLong, Binary::Add, i64::MAX as u64, 1, 2),
        (Int, Binary::Multiply, i32::MAX as u64, 2, 2),
        (LongLong, Binary::Multiply, i64::MAX as u64, 2, 2),
        (LongLong, Binary::Divide, 1u64 << 63, u64::MAX, 2),
        (Int, Binary::Divide, 1, 0, 1),
        (LongLong, Binary::ShiftLeft, 1, 63, 2),
        (Int, Binary::ShiftLeft, 1, 31, 2),
        (Int, Binary::ShiftLeft, 1, 32, 8),
        (LongLong, Binary::ShiftRight, 1, u64::MAX, 8),
    ] {
        let mut m = program(kind, op, a, b);
        let int = m.types.integer(Int);
        let f = &mut m.functions[0];
        f.values.push(int);
        f.blocks[0].instructions.push(Instruction::Convert {
            dst: ValueId(3),
            src: ValueId(2),
        });
        f.blocks[0].end = Terminator::Return(Some(ValueId(3)));
        f.signature = m.types.function(int, vec![], false, true).unwrap();
        assert_eq!(Native::new(&m, true).context().0, status, "{kind:?} {op:?}");
    }
    for result in [1, 2, 3, 7, -1, i32::MIN] {
        let mut m = program(Int, Binary::Add, 0, 0);
        m.functions[0].blocks[0].instructions = vec![Instruction::Constant {
            dst: ValueId(2),
            bits: (result as u32) as u64,
        }];
        assert_eq!(Native::new(&m, true).context(), (0, result));
    }
}
#[test]
fn signed_unsigned_shifts_unary_and_large_local_frames() {
    for kind in [Int, UnsignedInt, LongLong, UnsignedLongLong] {
        for count in [0, 1, 7, 15, kind.bits() - 2] {
            let expected = 1u64 << count;
            assert_eq!(
                call(&program(kind, Binary::ShiftLeft, 1, u64::from(count))),
                expected
            );
            let input = if kind.signed() {
                mask(kind.bits())
            } else {
                1u64 << (kind.bits() - 1)
            };
            let expected = if kind.signed() {
                u64::MAX
            } else {
                input >> count
            };
            assert_eq!(
                call(&program(kind, Binary::ShiftRight, input, u64::from(count))),
                expected
            );
        }
    }
    for (op, expected) in [
        (Unary::Plus, 5),
        (Unary::Negate, (-5i64) as u64),
        (Unary::Complement, (!5i64) as u64),
        (Unary::Not, 0),
    ] {
        let mut m = program(LongLong, Binary::Add, 5, 0);
        m.functions[0].blocks[0].instructions[2] = Instruction::Unary {
            dst: ValueId(2),
            op,
            src: ValueId(0),
        };
        if op == Unary::Not {
            let int = m.types.integer(Int);
            m.functions[0].values[2] = int;
            m.functions[0].signature = m.types.function(int, vec![], false, true).unwrap();
        }
        assert_eq!(call(&m), expected);
    }
    let mut m = program(Int, Binary::Add, 91, 0);
    let int = m.types.integer(Int);
    let f = &mut m.functions[0];
    f.locals = (0..600)
        .map(|i| Local {
            symbol: SymbolId(i + 1),
            ty: int,
        })
        .collect();
    f.blocks[0].instructions.extend([
        Instruction::InitializeLocal {
            local: LocalId(599),
            src: ValueId(2),
        },
        Instruction::ReadLocal {
            dst: ValueId(2),
            local: LocalId(599),
        },
    ]);
    assert_eq!(call(&m), 91);
    f_uninitialized(&mut m);
    assert_eq!(Native::new(&m, true).context().0, 3);
}
fn f_uninitialized(m: &mut Module) {
    m.functions[0].blocks[0].instructions = vec![Instruction::ReadLocal {
        dst: ValueId(2),
        local: LocalId(0),
    }];
}

#[test]
fn twelve_integer_parameters_cross_register_and_stack_boundaries() {
    let mut m = program(UnsignedLongLong, Binary::Add, 0, 0);
    let kinds = [
        Char,
        UnsignedChar,
        Short,
        UnsignedShort,
        Int,
        UnsignedInt,
        Long,
        UnsignedLong,
        LongLong,
        UnsignedLongLong,
        Char,
        LongLong,
    ];
    let parameter_types: Vec<_> = kinds.iter().map(|k| m.types.integer(*k)).collect();
    let ull = m.types.integer(UnsignedLongLong);
    let signature = m
        .types
        .function(ull, parameter_types.clone(), false, true)
        .unwrap();
    let f = &mut m.functions[0];
    f.signature = signature;
    f.parameters = (0..12).map(LocalId).collect();
    f.locals = parameter_types
        .iter()
        .enumerate()
        .map(|(i, ty)| Local {
            symbol: SymbolId(i + 1),
            ty: *ty,
        })
        .collect();
    f.values = vec![ull];
    let mut instructions = vec![Instruction::Constant {
        dst: ValueId(0),
        bits: 0,
    }];
    for (i, ty) in parameter_types.iter().enumerate() {
        let read = ValueId(f.values.len());
        f.values.push(*ty);
        let converted = ValueId(f.values.len());
        f.values.push(ull);
        instructions.extend([
            Instruction::ReadLocal {
                dst: read,
                local: LocalId(i),
            },
            Instruction::Convert {
                dst: converted,
                src: read,
            },
            Instruction::Binary {
                dst: ValueId(0),
                op: Binary::Add,
                lhs: ValueId(0),
                rhs: converted,
            },
        ]);
    }
    f.blocks[0].instructions = instructions;
    f.blocks[0].end = Terminator::Return(Some(ValueId(0)));
    let native = Native::new(&m, false);
    // SAFETY: twelve integer-position Windows ABI arguments; the generated body
    // reads the declared low width and normalizes its signedness on entry.
    let result = unsafe {
        let function: unsafe extern "system" fn(
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
        ) -> u64 = std::mem::transmute(native.address);
        function(
            255,
            255,
            65535,
            65535,
            u32::MAX as u64,
            u32::MAX as u64,
            u32::MAX as u64,
            u32::MAX as u64,
            u64::MAX,
            u64::MAX,
            255,
            7,
        )
    };
    let expected = (-1i128 + 255 - 1 + 65535 - 1 + i128::from(u32::MAX) - 1 + i128::from(u32::MAX)
        - 1
        + i128::from(u64::MAX)
        - 1
        + 7) as u64;
    assert_eq!(result, expected);
}

#[test]
fn independently_compiled_rust_caller_supplies_real_variadic_homes() {
    let source = "unsigned long long sum(int count,...){__builtin_va_list p;__builtin_va_start(p,count);unsigned long long sum=0;for(int i=0;i<count;i++)sum+=__builtin_va_arg(p,unsigned long long);__builtin_va_end(p);return sum;}";
    let unit =
        compiler::typed_front::parse(&format!("{source} int main(void){{return 0;}}")).unwrap();
    let mut module = compiler::typed_lower::lower(&unit).unwrap();
    module.functions.truncate(1);
    module.entry = None;
    let native = Native::new(&module, false);
    // SAFETY: verified scalar Windows-x64 variadic callee reads precisely count
    // u64 slots. A Rust-compiled caller supplies all register/stack positions.
    unsafe {
        let call: unsafe extern "system" fn(
            i32,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
            u64,
        ) -> u64 = std::mem::transmute(native.address);
        for count in 0..=11 {
            assert_eq!(
                call(count, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21),
                (count * count) as u64
            );
        }
    }
}
