//! Real nonblocking OS pipes exercise partial/zero-progress output paths.
#![cfg(all(windows, target_arch = "x86_64"))]
use compiler::ir::{Ir, Value};
use compiler::native_runtime::{RuntimeOptions, image};
use std::{
    ffi::c_void,
    io::Read,
    os::windows::io::BorrowedHandle,
    process::{Command, Stdio},
};
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open: u32,
        mode: u32,
        instances: u32,
        out_size: u32,
        in_size: u32,
        timeout: u32,
        security: *mut c_void,
    ) -> *mut c_void;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        creation: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn ConnectNamedPipe(pipe: *mut c_void, overlapped: *mut c_void) -> i32;
    fn GetNamedPipeInfo(
        pipe: *mut c_void,
        flags: *mut u32,
        out_size: *mut u32,
        in_size: *mut u32,
        instances: *mut u32,
    ) -> i32;
    fn PeekNamedPipe(
        pipe: *mut c_void,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        available: *mut u32,
        left: *mut u32,
    ) -> i32;
    fn WriteFile(
        file: *mut c_void,
        buffer: *const c_void,
        size: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn ReadFile(
        file: *mut c_void,
        buffer: *mut c_void,
        size: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 as isize != -1 {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
#[test]
fn nonblocking_byte_and_message_pipes_are_real_partial_write_failures() {
    for (case, mode) in [(0u32, 1u32), (1, 5)] {
        let name: Vec<u16> = format!("\\\\.\\pipe\\compiler-native-{}-{case}", std::process::id())
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: names are NUL-terminated UTF-16; handles are uniquely owned and all
        // buffers/out-parameters live for each synchronous Windows API call.
        let (server, client) = unsafe {
            let server = Handle(CreateNamedPipeW(
                name.as_ptr(),
                2,
                mode,
                1,
                4096,
                4096,
                0,
                std::ptr::null_mut(),
            ));
            assert_ne!(server.0 as isize, -1);
            let client = Handle(CreateFileW(
                name.as_ptr(),
                0x80000000,
                0,
                std::ptr::null_mut(),
                3,
                0,
                std::ptr::null_mut(),
            ));
            assert_ne!(client.0 as isize, -1);
            let connected = ConnectNamedPipe(server.0, std::ptr::null_mut());
            assert!(connected != 0 || GetLastError() == 535);
            (server, client)
        };
        let text = "Z".repeat(512 * 1024);
        let mut seeded = Vec::new();
        if case == 1 {
            unsafe {
                let mut quota = 0;
                assert_ne!(
                    GetNamedPipeInfo(
                        client.0,
                        std::ptr::null_mut(),
                        &mut quota,
                        std::ptr::null_mut(),
                        std::ptr::null_mut()
                    ),
                    0
                );
                assert!(quota > 0 && quota < 1024 * 1024);
                seeded = vec![b'Q'; quota as usize];
                let mut written = 0;
                assert_ne!(
                    WriteFile(
                        server.0,
                        seeded.as_ptr().cast(),
                        quota,
                        &mut written,
                        std::ptr::null_mut()
                    ),
                    0
                );
                assert_eq!(written, quota);
                written = u32::MAX;
                assert_ne!(
                    WriteFile(
                        server.0,
                        text.as_ptr().cast(),
                        4096,
                        &mut written,
                        std::ptr::null_mut()
                    ),
                    0
                );
                assert_eq!(
                    written, 0,
                    "full message pipe must genuinely report zero progress"
                );
            }
        }
        let ir = [
            Ir::Const(0, Value::Str(text.clone())),
            Ir::Print("%s".into(), vec![0], 1),
            Ir::Halt,
        ];
        let native = image(&ir, &RuntimeOptions::default()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "native pipe output {} {case}.exe",
            std::process::id()
        ));
        std::fs::write(&path, &native.bytes).unwrap();
        let mut command = Command::new(&path);
        command.stdin(Stdio::null()).stderr(Stdio::piped());
        // Keep the original server endpoint alive until queued bytes are read.
        // SAFETY: the borrow remains within the owned server handle lifetime;
        // try_clone_to_owned creates a distinct handle for child redirection.
        let child_server = unsafe { BorrowedHandle::borrow_raw(server.0) }
            .try_clone_to_owned()
            .unwrap();
        command.stdout(Stdio::from(child_server));
        let mut child = command.spawn().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut bytes = vec![];
            stderr.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let start = std::time::Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed().as_secs() >= 5 {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("nonblocking pipe program timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let error = reader.join().unwrap();
        assert_eq!(status.code(), Some(7));
        assert!(String::from_utf8_lossy(&error).contains("IO failure"));
        unsafe {
            let mut available = 0;
            assert_ne!(
                PeekNamedPipe(
                    client.0,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut()
                ),
                0
            );
            if case == 0 {
                assert!(
                    available > 0 && available < text.len() as u32,
                    "expected a real partial total transfer, got {available}"
                );
            } else {
                assert_eq!(
                    available,
                    seeded.len() as u32,
                    "child must not add data to the full message pipe"
                );
            }
            let mut bytes = vec![0; available as usize];
            let mut read = 0;
            assert_ne!(
                ReadFile(
                    client.0,
                    bytes.as_mut_ptr().cast(),
                    available,
                    &mut read,
                    std::ptr::null_mut()
                ),
                0
            );
            assert_eq!(read, available);
            if case == 0 {
                assert_eq!(bytes, &text.as_bytes()[..available as usize]);
            } else {
                assert_eq!(bytes, seeded);
            }
        }
        drop(command);
        drop(server);
        std::fs::remove_file(path).unwrap();
    }
}
