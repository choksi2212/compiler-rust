//! Independent structural checks use serialized bytes, not writer layout constants.
use compiler::native_pe::{FoundationLayout, Rva, Va, image};
fn word(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn dword(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn qword(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn check(b: &[u8], payload: &[u8]) {
    assert_eq!(&b[..2], b"MZ");
    assert_eq!(dword(b, 60), 128);
    assert_eq!(&b[128..132], b"PE\0\0");
    assert_eq!(word(b, 132), 0x8664);
    assert_eq!(word(b, 134), 2);
    assert_eq!(dword(b, 136), 0);
    assert_eq!(dword(b, 140), 0);
    assert_eq!(dword(b, 144), 0);
    assert_eq!(word(b, 148), 240);
    assert_eq!(word(b, 150), 0x23);
    let o = 152;
    assert_eq!(word(b, o), 0x20b);
    assert_eq!(qword(b, o + 24), 0x140000000);
    assert_eq!(dword(b, o + 16), 4096);
    assert_eq!(dword(b, o + 20), 4096);
    assert_eq!(dword(b, o + 32), 4096);
    assert_eq!(dword(b, o + 36), 512);
    assert_eq!(dword(b, o + 56), 12288);
    assert_eq!(dword(b, o + 60), 512);
    assert_eq!(word(b, o + 68), 3);
    assert_eq!(word(b, o + 70), 0x100);
    assert_eq!(dword(b, o + 108), 16);
    for (offset, value) in [(72, 1048576), (80, 4096), (88, 1048576), (96, 4096)] {
        assert_eq!(qword(b, o + offset), value);
    }
    for i in 0..16 {
        let dir = o + 112 + i * 8;
        match i {
            1 => {
                assert_eq!(dword(b, dir), 8192);
                assert_eq!(dword(b, dir + 4), 40);
            }
            12 => {
                assert_eq!(dword(b, dir), 8272);
                assert_eq!(dword(b, dir + 4), 16);
            }
            _ => assert_eq!(qword(b, dir), 0),
        }
    }
    // Audit reserved/default fields and version choices independently.
    for (at, len) in [
        (o + 2, 2),
        (o + 12, 4),
        (o + 42, 6),
        (o + 50, 6),
        (o + 64, 4),
        (o + 104, 4),
        (416, 12),
        (456, 12),
    ] {
        assert!(
            b[at..at + len].iter().all(|v| *v == 0),
            "nonzero reserved/default field at {at}"
        );
    }
    assert_eq!(word(b, o + 40), 6);
    assert_eq!(word(b, o + 48), 6);
    assert!(b[472..512].iter().all(|v| *v == 0));
    let text = 392;
    let imports = 432;
    let raw = payload.len().div_ceil(512) * 512;
    assert_eq!(&b[text..text + 8], b".text\0\0\0");
    assert_eq!(dword(b, text + 8), payload.len() as u32);
    assert_eq!(dword(b, text + 12), 4096);
    assert_eq!(dword(b, text + 16), raw as u32);
    assert_eq!(dword(b, text + 20), 512);
    assert_eq!(dword(b, text + 36), 0x60000020);
    assert_eq!(&b[imports..imports + 8], b".idata\0\0");
    assert_eq!(dword(b, imports + 8), 256);
    assert_eq!(dword(b, imports + 12), 8192);
    assert_eq!(dword(b, imports + 16), 512);
    let data = dword(b, imports + 20) as usize;
    assert_eq!(data, 512 + raw);
    assert_eq!(dword(b, imports + 36), 0xc0000040);
    assert_eq!(b.len(), data + 512);
    assert_eq!(&b[512..512 + payload.len()], payload);
    assert!(b[512 + payload.len()..data].iter().all(|v| *v == 0));
    assert_eq!(dword(b, o + 4), raw as u32);
    assert_eq!(dword(b, o + 8), 512);
    assert_eq!(dword(b, data), 8256);
    assert_eq!(dword(b, data + 4), 0);
    assert_eq!(dword(b, data + 8), 0);
    assert_eq!(dword(b, data + 12), 8288);
    assert_eq!(dword(b, data + 16), 8272);
    assert!(b[data + 20..data + 40].iter().all(|v| *v == 0));
    assert_eq!(qword(b, data + 64), 8320);
    assert_eq!(qword(b, data + 72), 0);
    assert_eq!(qword(b, data + 80), 8320);
    assert_eq!(qword(b, data + 88), 0);
    assert_eq!(&b[data + 96..data + 109], b"KERNEL32.dll\0");
    assert_eq!(word(b, data + 128), 0);
    assert_eq!(&b[data + 130..data + 142], b"ExitProcess\0");
}
#[test]
fn all_supported_code_sizes_and_padding() {
    for n in 1usize..=4096 {
        let payload: Vec<u8> = (0..n).map(|i| (i.wrapping_mul(73) % 256) as u8).collect();
        check(&image(&payload).unwrap(), &payload);
    }
}
#[test]
fn invalid_sizes_and_address_domains() {
    for n in [0, 4097, usize::MAX] {
        assert!(FoundationLayout::for_code_size(n).is_err());
    }
    assert!(Rva(1).at_base(Va(u64::MAX)).is_err());
    assert_eq!(Rva(4096).at_base(Va(0x140000000)).unwrap(), Va(0x140001000));
}
#[cfg(all(windows, target_arch = "x86_64"))]
#[test]
fn executes_padded_images_at_boundaries() {
    let dir = std::env::temp_dir().join(format!("PE boundary process {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for n in [15, 511, 512, 513, 4095, 4096] {
        let mut text = vec![0x90; n - 15];
        text.extend([0x48, 0x83, 0xec, 0x28, 0xb9]);
        text.extend(37u32.to_le_bytes());
        text.extend([0xff, 0x15]);
        let next = 4096 + text.len() as i32 + 4;
        text.extend((8272i32 - next).to_le_bytes());
        assert_eq!(text.len(), n);
        let path = dir.join(format!("boundary {n}.exe"));
        std::fs::write(&path, image(&text).unwrap()).unwrap();
        let mut child = std::process::Command::new(&path).spawn().unwrap();
        let start = std::time::Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break s;
            }
            if start.elapsed().as_secs() >= 5 {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("timeout at size {n}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(37));
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(dir).unwrap();
}
