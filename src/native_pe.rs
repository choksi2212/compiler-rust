//! Project-owned PE32+ executable writer. No external assembler or linker.
//! Foundation stage: a single code section and a Kernel32 ExitProcess import.
/// Relative address in a loaded image; deliberately distinct from file offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rva(pub u32);
/// Byte offset in the serialized image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOffset(pub u32);
/// Full address including the image base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Va(pub u64);
impl Rva {
    pub fn at_base(self, base: Va) -> Result<Va, String> {
        base.0
            .checked_add(self.0 as u64)
            .map(Va)
            .ok_or("image address overflow".into())
    }
}
/// The constrained foundation layout, not the future general PE layout engine.
#[derive(Debug, Clone, Copy)]
pub struct FoundationLayout {
    pub code_start: FileOffset,
    pub import_start: FileOffset,
    pub code_rva: Rva,
    pub import_rva: Rva,
    pub code_raw_size: u32,
    pub file_size: u32,
}
impl FoundationLayout {
    pub fn for_code_size(size: usize) -> Result<Self, String> {
        if !(1..=4096).contains(&size) {
            return Err("native code must contain 1..4096 bytes".into());
        }
        let raw = (u32::try_from(size).map_err(|_| "code size overflow")? + 511) & !511;
        Ok(Self {
            code_start: FileOffset(512),
            import_start: FileOffset(512 + raw),
            code_rva: Rva(4096),
            import_rva: Rva(8192),
            code_raw_size: raw,
            file_size: 1024 + raw,
        })
    }
}
pub const TEXT_RVA: u32 = 0x1000;
pub const EXIT_IAT_RVA: u32 = 0x2050;
fn u16at(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn u32at(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn u64at(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
/// Package native entrypoint instructions; caller supplies x64 machine code.
/// The entrypoint may call ExitProcess indirectly through EXIT_IAT_RVA.
/// This foundation supports at most one page of code and has no relocations.
pub fn image(text: &[u8]) -> Result<Vec<u8>, String> {
    let layout = FoundationLayout::for_code_size(text.len())?;
    let raw_size = layout.code_raw_size as usize;
    let data_offset = layout.import_start.0 as usize;
    let mut b = vec![0u8; layout.file_size as usize];
    b[..2].copy_from_slice(b"MZ");
    u32at(&mut b, 0x3c, 0x80);
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let c = 0x84;
    u16at(&mut b, c, 0x8664);
    u16at(&mut b, c + 2, 2);
    u16at(&mut b, c + 16, 240);
    u16at(&mut b, c + 18, 0x23);
    let o = c + 20;
    u16at(&mut b, o, 0x20b);
    u32at(&mut b, o + 4, raw_size as u32);
    u32at(&mut b, o + 8, 0x200);
    u32at(&mut b, o + 16, TEXT_RVA);
    u32at(&mut b, o + 20, TEXT_RVA);
    u64at(&mut b, o + 24, 0x140000000);
    u32at(&mut b, o + 32, 0x1000);
    u32at(&mut b, o + 36, 0x200);
    u16at(&mut b, o + 40, 6);
    u16at(&mut b, o + 48, 6);
    u32at(&mut b, o + 56, 0x3000);
    u32at(&mut b, o + 60, 0x200);
    u16at(&mut b, o + 68, 3);
    u16at(&mut b, o + 70, 0x100);
    for (off, value) in [(72, 0x100000), (80, 0x1000), (88, 0x100000), (96, 0x1000)] {
        u64at(&mut b, o + off, value);
    }
    u32at(&mut b, o + 108, 16);
    u32at(&mut b, o + 120, 0x2000);
    u32at(&mut b, o + 124, 40);
    u32at(&mut b, o + 208, EXIT_IAT_RVA);
    u32at(&mut b, o + 212, 16);
    let section = o + 240;
    b[section..section + 5].copy_from_slice(b".text");
    u32at(&mut b, section + 8, text.len() as u32);
    u32at(&mut b, section + 12, TEXT_RVA);
    u32at(&mut b, section + 16, raw_size as u32);
    u32at(&mut b, section + 20, 0x200);
    u32at(&mut b, section + 36, 0x60000020);
    let section = section + 40;
    b[section..section + 6].copy_from_slice(b".idata");
    u32at(&mut b, section + 8, 0x100);
    u32at(&mut b, section + 12, 0x2000);
    u32at(&mut b, section + 16, 0x200);
    u32at(&mut b, section + 20, data_offset as u32);
    u32at(&mut b, section + 36, 0xc0000040);
    b[0x200..0x200 + text.len()].copy_from_slice(text);
    u32at(&mut b, data_offset, 0x2040);
    u32at(&mut b, data_offset + 12, 0x2060);
    u32at(&mut b, data_offset + 16, EXIT_IAT_RVA);
    u64at(&mut b, data_offset + 0x40, 0x2080);
    u64at(&mut b, data_offset + 0x50, 0x2080);
    b[data_offset + 0x60..data_offset + 0x6d].copy_from_slice(b"KERNEL32.dll\0");
    b[data_offset + 0x82..data_offset + 0x8e].copy_from_slice(b"ExitProcess\0");
    Ok(b)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn exit_code(code: u32) -> Vec<u8> {
        // sub rsp,40; mov ecx,code; call qword ptr [rip + IAT displacement].
        let mut text = vec![0x48, 0x83, 0xec, 0x28, 0xb9];
        text.extend(code.to_le_bytes());
        text.extend([0xff, 0x15]);
        let next = TEXT_RVA + text.len() as u32 + 4;
        text.extend((EXIT_IAT_RVA as i32 - next as i32).to_le_bytes());
        image(&text).unwrap()
    }
    #[test]
    fn headers_and_bounds() {
        let b = exit_code(0);
        assert_eq!(&b[..2], b"MZ");
        assert_eq!(&b[0x80..0x84], b"PE\0\0");
        assert_eq!(
            u16::from_le_bytes(b[0x84..0x86].try_into().unwrap()),
            0x8664
        );
        assert_eq!(b.len(), 0x600);
        assert!(image(&[]).is_err());
        assert!(image(&vec![0; 4097]).is_err());
        let large = image(&vec![0x90; 4096]).unwrap();
        assert_eq!(large.len(), 0x1400);
    }
    #[cfg(all(windows, target_arch = "x86_64"))]
    #[test]
    fn windows_executes_generated_machine_code() {
        let dir = std::env::temp_dir().join(format!("compiler native PE {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for code in [0u32, 37, 255] {
            let path = dir.join(format!("exit {code}.exe"));
            std::fs::write(&path, exit_code(code)).unwrap();
            let mut process = std::process::Command::new(&path).spawn().unwrap();
            let start = std::time::Instant::now();
            let status = loop {
                if let Some(status) = process.try_wait().unwrap() {
                    break status;
                }
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    process.kill().unwrap();
                    process.wait().unwrap();
                    panic!("native process timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!(status.code(), Some(code as i32));
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }
}
