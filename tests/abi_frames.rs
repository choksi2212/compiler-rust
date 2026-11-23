use compiler::abi::*;
use compiler::x64::*;
#[test]
fn frame_layout_and_unwind_fixtures() {
    let f = FramePlan::new(&FrameRequest {
        has_calls: true,
        saved: vec![Reg::new(3).unwrap()],
        ..FrameRequest::default()
    })
    .unwrap();
    assert_eq!(f.allocation(), 32);
    assert_eq!(f.outgoing_size(), 32);
    let p = f.prologue(Label(100)).unwrap();
    assert_eq!(p.unwind, vec![1, 8, 2, 0, 8, 0x32, 1, 0x30]);
    assert_eq!(f.incoming_stack_offset(4).unwrap(), 80);
    for saved in 0..=8 {
        let all = [3, 5, 6, 7, 12, 13, 14, 15];
        for arguments in 0..=12 {
            for local in [8, 128, 4096, 65536] {
                let request = FrameRequest {
                    has_calls: true,
                    max_call_arguments: arguments,
                    locals: vec![(local, 8), (16, 16)],
                    temporaries: 4,
                    initialization_flags: 7,
                    saved: all[..saved].iter().map(|n| Reg::new(*n).unwrap()).collect(),
                };
                let f = FramePlan::new(&request).unwrap();
                assert_eq!(
                    (8i64 - saved as i64 * 8 - f.allocation() as i64).rem_euclid(16),
                    0
                );
                assert!(f.outgoing_size() >= 32);
                assert_eq!(f.local_offsets()[1] % 16, 0);
                assert!(f.flag_offsets()[6] < f.allocation());
                let p = f.prologue(Label(100)).unwrap();
                assert_eq!(p.unwind.len() % 4, 0);
                assert!(p.byte_length > 0);
            }
        }
    }
    for request in [
        FrameRequest {
            saved: vec![Reg::RAX],
            ..FrameRequest::default()
        },
        FrameRequest {
            saved: vec![Reg::RBP, Reg::RBP],
            ..FrameRequest::default()
        },
        FrameRequest {
            has_calls: true,
            max_call_arguments: usize::MAX,
            ..FrameRequest::default()
        },
        FrameRequest {
            locals: vec![(8, 3)],
            ..FrameRequest::default()
        },
        FrameRequest {
            locals: vec![(u32::MAX, 8)],
            ..FrameRequest::default()
        },
        FrameRequest {
            temporaries: usize::MAX,
            ..FrameRequest::default()
        },
    ] {
        assert!(FramePlan::new(&request).is_err());
    }
    assert!(scalar_argument(usize::MAX).is_err());
    assert!(
        function_table(&[RuntimeFunction {
            begin: 5,
            end: 4,
            unwind: 4
        }])
        .is_err()
    );
    assert!(
        function_table(&[
            RuntimeFunction {
                begin: 0,
                end: 5,
                unwind: 4
            },
            RuntimeFunction {
                begin: 4,
                end: 6,
                unwind: 8
            }
        ])
        .is_err()
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU64, Ordering};
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
        fn RtlLookupFunctionEntry(
            pc: u64,
            base: *mut u64,
            history: *mut c_void,
        ) -> *const RuntimeFunction;
        fn RtlCaptureContext(context: *mut Context);
        fn RtlVirtualUnwind(
            kind: u32,
            base: u64,
            pc: u64,
            function: *const RuntimeFunction,
            context: *mut Context,
            handler: *mut *mut c_void,
            frame: *mut u64,
            pointers: *mut c_void,
        ) -> *mut c_void;
    }
    #[repr(C, align(16))]
    struct Context {
        bytes: [u8; 1232],
    }
    impl Context {
        fn read(&self, at: usize) -> u64 {
            u64::from_le_bytes(self.bytes[at..at + 8].try_into().unwrap())
        }
    }
    struct Native {
        address: *mut c_void,
        table: Vec<RuntimeFunction>,
    }
    impl Native {
        fn new(instructions: &[Instruction], frames: &[(usize, usize, Vec<u8>)]) -> Self {
            let f = encode(instructions).unwrap();
            assert!(f.bytes.len() < 4096);
            let mut metadata = vec![];
            let mut table = vec![];
            for (begin, end, unwind) in frames {
                while metadata.len() % 4 != 0 {
                    metadata.push(0);
                }
                table.push(RuntimeFunction {
                    begin: *begin as u32,
                    end: *end as u32,
                    unwind: 4096 + metadata.len() as u32,
                });
                metadata.extend(unwind);
            }
            assert!(metadata.len() < 4096);
            function_table(&table).unwrap();
            // SAFETY: allocate two owned pages, copy bounded code/metadata, then make only
            // the code page executable. The function table and allocation outlive calls.
            unsafe {
                let address = VirtualAlloc(std::ptr::null_mut(), 8192, 0x3000, 4);
                assert!(!address.is_null());
                std::ptr::copy_nonoverlapping(
                    f.bytes.as_ptr(),
                    address.cast::<u8>(),
                    f.bytes.len(),
                );
                std::ptr::copy_nonoverlapping(
                    metadata.as_ptr(),
                    address.cast::<u8>().add(4096),
                    metadata.len(),
                );
                let mut old = 0;
                assert_ne!(VirtualProtect(address, 4096, 0x20, &mut old), 0);
                assert_ne!(
                    RtlAddFunctionTable(table.as_ptr(), table.len() as u32, address as u64),
                    0
                );
                Self { address, table }
            }
        }
        fn call(&self) -> u64 {
            // SAFETY: the owned encoder emits a zero-argument Windows-ABI entrypoint,
            // preserves nonvolatile state, and its pages/table remain registered.
            unsafe {
                let function: unsafe extern "system" fn() -> u64 =
                    std::mem::transmute(self.address);
                function()
            }
        }
    }
    impl Drop for Native {
        fn drop(&mut self) {
            unsafe {
                assert_ne!(RtlDeleteFunctionTable(self.table.as_ptr()), 0);
                assert_ne!(VirtualFree(self.address, 0, 0x8000), 0);
            }
        }
    }
    fn offset(instructions: &[Instruction], label: Label) -> usize {
        encode(instructions)
            .unwrap()
            .listing
            .iter()
            .find(|r| matches!(r.instruction,Instruction::Bind(l)if l==label))
            .unwrap()
            .offset
    }
    #[test]
    fn native_scalar_arguments_and_nonvolatile_preservation() {
        let rbx = Reg::new(3).unwrap();
        let r12 = Reg::new(12).unwrap();
        let scratch = Reg::new(10).unwrap();
        for count in 0..=12 {
            let root = FramePlan::new(&FrameRequest {
                has_calls: true,
                max_call_arguments: count,
                saved: vec![rbx, r12],
                ..FrameRequest::default()
            })
            .unwrap();
            let callee = FramePlan::new(&FrameRequest {
                saved: [3, 5, 6, 7, 12, 13, 14, 15]
                    .iter()
                    .map(|n| Reg::new(*n).unwrap())
                    .collect(),
                ..FrameRequest::default()
            })
            .unwrap();
            let rp = root.prologue(Label(100)).unwrap();
            let cp = callee.prologue(Label(101)).unwrap();
            let mut c = rp.instructions.clone();
            c.extend([
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: rbx,
                    value: 99,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: r12,
                    value: 100,
                },
            ]);
            for i in 0..count {
                let reg = match scalar_argument(i).unwrap() {
                    ArgumentLocation::Register(r) => r,
                    ArgumentLocation::Stack(_) => scratch,
                };
                c.push(Instruction::MovImm {
                    width: Width::Qword,
                    dst: reg,
                    value: i as u64 + 1,
                });
                if let ArgumentLocation::Stack(at) = scalar_argument(i).unwrap() {
                    c.push(Instruction::Store {
                        width: Width::Qword,
                        address: Address::base(Reg::RSP, at as i32),
                        src: reg,
                    });
                }
            }
            c.extend([
                Instruction::Call(Label(1)),
                Instruction::Immediate {
                    width: Width::Qword,
                    op: Immediate::Compare,
                    reg: rbx,
                    value: 99,
                },
                Instruction::Branch {
                    condition: Condition::NotEqual,
                    label: Label(2),
                },
                Instruction::Immediate {
                    width: Width::Qword,
                    op: Immediate::Compare,
                    reg: r12,
                    value: 100,
                },
                Instruction::Branch {
                    condition: Condition::NotEqual,
                    label: Label(2),
                },
                Instruction::Jump(Label(3)),
                Instruction::Bind(Label(2)),
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: Reg::RAX,
                    value: u64::MAX,
                },
                Instruction::Bind(Label(3)),
            ]);
            c.extend(root.epilogue());
            c.push(Instruction::Bind(Label(1)));
            c.extend(cp.instructions.clone());
            c.push(Instruction::MovImm {
                width: Width::Qword,
                dst: Reg::RAX,
                value: 0,
            });
            for i in 0..count {
                let reg = match scalar_argument(i).unwrap() {
                    ArgumentLocation::Register(r) => r,
                    ArgumentLocation::Stack(_) => {
                        c.push(Instruction::Load {
                            width: Width::Qword,
                            dst: scratch,
                            address: Address::base(
                                Reg::RSP,
                                callee.incoming_stack_offset(i).unwrap() as i32,
                            ),
                        });
                        scratch
                    }
                };
                c.push(Instruction::Binary {
                    width: Width::Qword,
                    op: Binary::Add,
                    dst: Reg::RAX,
                    src: reg,
                });
            }
            c.extend([
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: rbx,
                    value: 7,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: r12,
                    value: 8,
                },
            ]);
            c.extend(callee.epilogue());
            let split = offset(&c, Label(1));
            let end = encode(&c).unwrap().bytes.len();
            let native = Native::new(&c, &[(0, split, rp.unwind), (split, end, cp.unwind)]);
            assert_eq!(native.call(), (count * (count + 1) / 2) as u64);
        }
    }
    static START: AtomicU64 = AtomicU64::new(0);
    static END: AtomicU64 = AtomicU64::new(0);
    static RETURN: AtomicU64 = AtomicU64::new(0);
    #[inline(never)]
    unsafe extern "system" fn inspect_real_frame(expected_sp: u64) -> u64 {
        let mut context = Context { bytes: [0; 1232] };
        // SAFETY: capture into a correctly sized/aligned Windows x64 CONTEXT. Only
        // a copied context is unwound; live native frames/table remain allocated.
        unsafe {
            RtlCaptureContext(&mut context);
            for _ in 0..8 {
                let pc = context.read(248);
                let mut base = 0;
                let function = RtlLookupFunctionEntry(pc, &mut base, std::ptr::null_mut());
                if function.is_null() {
                    return 0;
                }
                let target = pc >= START.load(Ordering::SeqCst) && pc < END.load(Ordering::SeqCst);
                let mut handler = std::ptr::null_mut();
                let mut frame = 0;
                RtlVirtualUnwind(
                    0,
                    base,
                    pc,
                    function,
                    &mut context,
                    &mut handler,
                    &mut frame,
                    std::ptr::null_mut(),
                );
                if target {
                    return u64::from(
                        context.read(144) == 0x123456789abcdef
                            && context.read(152) == expected_sp
                            && context.read(248) == RETURN.load(Ordering::SeqCst),
                    );
                }
            }
        }
        0
    }
    #[test]
    fn os_unwinds_actual_generated_small_and_probed_large_frames() {
        let rbx = Reg::new(3).unwrap();
        let scratch = Reg::new(10).unwrap();
        for local_bytes in [8, 8240, 65536, 600000] {
            let root = FramePlan::new(&FrameRequest {
                has_calls: true,
                saved: vec![rbx],
                ..FrameRequest::default()
            })
            .unwrap();
            let callee = FramePlan::new(&FrameRequest {
                has_calls: true,
                locals: vec![(local_bytes, 8)],
                saved: vec![rbx],
                ..FrameRequest::default()
            })
            .unwrap();
            let rp = root.prologue(Label(100)).unwrap();
            let cp = callee.prologue(Label(101)).unwrap();
            let mut c = rp.instructions.clone();
            c.extend([
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: rbx,
                    value: 0x123456789abcdef,
                },
                Instruction::Mov {
                    width: Width::Qword,
                    dst: Reg::RCX,
                    src: Reg::RSP,
                },
                Instruction::Call(Label(1)),
                Instruction::Bind(Label(2)),
                Instruction::Nop,
            ]);
            c.extend(root.epilogue());
            c.push(Instruction::Bind(Label(1)));
            c.extend(cp.instructions.clone());
            c.extend([
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: rbx,
                    value: 42,
                },
                Instruction::MovImm {
                    width: Width::Qword,
                    dst: scratch,
                    value: inspect_real_frame as *const () as usize as u64,
                },
                Instruction::CallReg(scratch),
                Instruction::Nop,
            ]);
            c.extend(callee.epilogue());
            let split = offset(&c, Label(1));
            let end = encode(&c).unwrap().bytes.len();
            let native = Native::new(&c, &[(0, split, rp.unwind), (split, end, cp.unwind)]);
            START.store(native.address as u64 + split as u64, Ordering::SeqCst);
            END.store(native.address as u64 + end as u64, Ordering::SeqCst);
            RETURN.store(
                native.address as u64 + offset(&c, Label(2)) as u64,
                Ordering::SeqCst,
            );
            assert_eq!(native.call(), 1, "frame allocation {}", callee.allocation());
        }
    }
}
#[test]
fn unwind_allocation_encoding_boundaries() {
    for bytes in [8, 128, 136, 4096, 524280, 524288, 600000, 1048576] {
        let request = FrameRequest {
            locals: vec![(bytes, 8)],
            saved: vec![Reg::new(3).unwrap()],
            ..FrameRequest::default()
        };
        let frame = FramePlan::new(&request).unwrap();
        let p = frame.prologue(Label(100)).unwrap();
        validate_unwind(&p.unwind).unwrap();
        let opcode = p.unwind[5] & 15;
        let info = p.unwind[5] >> 4;
        assert_eq!(opcode, if frame.allocation() <= 128 { 2 } else { 1 });
        if frame.allocation() > 128 {
            assert_eq!(
                info,
                if frame.allocation() / 8 <= u16::MAX as u32 {
                    0
                } else {
                    1
                }
            );
        }
    }
}
