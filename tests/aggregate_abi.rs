#![cfg(all(windows, target_arch = "x86_64"))]
//! Independent Rust-compiled callers exercise the emitted Microsoft C ABI.
use compiler::{
    abi::RuntimeFunction,
    typed_front, typed_ir, typed_lower, typed_native,
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

struct Executable {
    address: *mut c_void,
    table: Box<RuntimeFunction>,
}
impl Executable {
    fn new(source: &str) -> Self {
        let unit = typed_front::parse(&format!("{source} int main(void){{return 0;}}")).unwrap();
        let mut module = typed_lower::lower(&unit).unwrap();
        module.functions.truncate(1);
        module.entry = None;
        typed_ir::verify(&module).unwrap();
        let function = typed_native::lower(&module, &mut LabelAllocator::default(), false).unwrap();
        let text = encode(&function.instructions).unwrap();
        assert!(text.external_fixups.is_empty());
        let pages = text.bytes.len().div_ceil(4096) * 4096;
        assert!(function.unwind.len() < 4096);
        let table = Box::new(RuntimeFunction {
            begin: 0,
            end: u32::try_from(text.bytes.len()).unwrap(),
            unwind: u32::try_from(pages).unwrap(),
        });
        // SAFETY: code and metadata remain allocated until Drop. The code pages
        // become RX, the metadata page stays RW, and Windows registers the real
        // frame's unwind data before any emitted instruction executes.
        unsafe {
            let address = VirtualAlloc(std::ptr::null_mut(), pages + 4096, 0x3000, 4);
            assert!(!address.is_null());
            std::ptr::copy_nonoverlapping(text.bytes.as_ptr(), address.cast(), text.bytes.len());
            std::ptr::copy_nonoverlapping(
                function.unwind.as_ptr(),
                address.cast::<u8>().add(pages),
                function.unwind.len(),
            );
            let mut old = 0;
            assert_ne!(VirtualProtect(address, pages, 0x20, &mut old), 0);
            assert_ne!(
                FlushInstructionCache((-1isize) as *mut c_void, address, text.bytes.len()),
                0
            );
            assert_ne!(RtlAddFunctionTable(&*table, 1, address as u64), 0);
            Self { address, table }
        }
    }
}
impl Drop for Executable {
    fn drop(&mut self) {
        // SAFETY: no call remains active; this owns both registrations/storage.
        unsafe {
            assert_ne!(RtlDeleteFunctionTable(&*self.table), 0);
            assert_ne!(VirtualFree(self.address, 0, 0x8000), 0);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bytes<const N: usize> {
    bytes: [u8; N],
}

fn argument_and_return<const N: usize>() {
    let source = format!(
        "struct R{{unsigned char bytes[{N}];}};struct R process(int p0,int p1,struct R value,int p2,int p3,int p4,int p5,int p6){{value.bytes[0]+=p0+p1+p2+p3+p4+p5+p6;value.bytes[{}]^=90;return value;}}",
        N - 1
    );
    let executable = Executable::new(&source);
    let original = Bytes {
        bytes: std::array::from_fn(|i| 10 + i as u8 * 3),
    };
    let mut expected = original;
    expected.bytes[0] = expected.bytes[0].wrapping_add(28);
    expected.bytes[N - 1] ^= 90;
    // SAFETY: repr(C) makes this Rust-compiled caller independent of our caller
    // lowering. Its real signature matches the verified C definition. For
    // non-register-size records Rust supplies the hidden result and by-value
    // pointer, shifting the aggregate and following scalars across R9/stack.
    let result = unsafe {
        let call: unsafe extern "system" fn(
            i32,
            i32,
            Bytes<N>,
            i32,
            i32,
            i32,
            i32,
            i32,
        ) -> Bytes<N> = std::mem::transmute(executable.address);
        call(1, 2, original, 3, 4, 5, 6, 7)
    };
    assert_eq!(result, expected, "record extent {N}");
    assert_eq!(
        original.bytes,
        std::array::from_fn(|i| 10 + i as u8 * 3),
        "by-value parameter must preserve the caller's source record"
    );
}

#[test]
fn rust_caller_covers_all_register_widths_and_irregular_hidden_records() {
    argument_and_return::<1>();
    argument_and_return::<2>();
    argument_and_return::<4>();
    argument_and_return::<8>();
    argument_and_return::<3>();
    argument_and_return::<5>();
    argument_and_return::<12>();
    argument_and_return::<24>();
}
