//! Bounded input and transactional publication for the Windows target.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn read_source(path: &Path) -> Result<(PathBuf, String), String> {
    let resolved = fs::canonicalize(path).map_err(fail)?;
    let file = File::open(&resolved).map_err(fail)?;
    regular(&file)?;
    let mut bytes = vec![];
    file.take(crate::lexer::MAX_SOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() > crate::lexer::MAX_SOURCE_BYTES {
        return Err("source exceeds16MiB limit".into());
    }
    let text = String::from_utf8(bytes).map_err(|_| "source must be UTF-8".to_string())?;
    Ok((resolved, text))
}

fn regular(file: &File) -> Result<(), String> {
    if !file.metadata().map_err(fail)?.is_file() {
        return Err("expected a regular disk file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        if unsafe { GetFileType(file.as_raw_handle()) } != 1 {
            return Err("expected a regular disk file".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Information {
    volume: u64,
    id: [u8; 16],
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandleEx(
        handle: *mut std::ffi::c_void,
        class: i32,
        info: *mut Information,
        size: u32,
    ) -> i32;
    fn GetFileType(handle: *mut std::ffi::c_void) -> u32;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
fn identity(path: &Path) -> Result<(u64, [u8; 16]), String> {
    use std::os::windows::io::AsRawHandle;
    let file = File::open(path).map_err(fail)?;
    regular(&file)?;
    let mut info = Information::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18,
            &mut info,
            std::mem::size_of::<Information>() as u32,
        )
    } == 0
    {
        return Err(fail(std::io::Error::last_os_error()));
    }
    Ok((info.volume, info.id))
}

#[cfg(not(windows))]
fn identity(_path: &Path) -> Result<(u64, [u8; 16]), String> {
    Err("native publication requires a Windows host".into())
}

fn valid_name(name: &std::ffi::OsStr) -> Result<(), String> {
    let text = name.to_string_lossy();
    let base = text
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&base.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|n| {
                ["1", "2", "3", "4", "5", "6", "7", "8", "9", "¹", "²", "³"].contains(&n)
            })
        });
    if text.is_empty()
        || text.ends_with(['.', ' '])
        || text.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
        || reserved
    {
        return Err("invalid or reserved Windows output filename".into());
    }
    Ok(())
}

pub struct OutputTarget {
    path: PathBuf,
    protected: Vec<(PathBuf, (u64, [u8; 16]))>,
    overwrite: bool,
}
impl OutputTarget {
    pub fn prepare(path: &Path, protected: &[PathBuf], overwrite: bool) -> Result<Self, String> {
        // Path::file_name normalizes trailing separators; reject their original spelling too.
        if path.as_os_str().to_string_lossy().ends_with(['/', '\\']) {
            return Err("output must name a file".into());
        }
        let name = path.file_name().ok_or("output must name a file")?;
        valid_name(name)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let path = fs::canonicalize(parent).map_err(fail)?.join(name);
        let mut identities = vec![];
        for input in protected {
            let input = fs::canonicalize(input).map_err(fail)?;
            let id = identity(&input)?;
            identities.push((input, id));
        }
        let target = Self {
            path,
            protected: identities,
            overwrite,
        };
        target.check()?;
        Ok(target)
    }

    fn check(&self) -> Result<(), String> {
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {
                let resolved = fs::canonicalize(&self.path).map_err(fail)?;
                let id = identity(&self.path)?;
                if self
                    .protected
                    .iter()
                    .any(|(p, i)| *p == resolved || *i == id)
                {
                    return Err("output aliases a protected input or the running compiler".into());
                }
                if !self.overwrite {
                    return Err("output already exists; use --overwrite to replace it".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(fail(e)),
        }
        Ok(())
    }

    pub fn publish(&self, bytes: &[u8]) -> Result<(), String> {
        crate::pe::validate(bytes)?;
        self.check()?;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let mut pending = None;
        for _ in 0..128 {
            let path = self.path.with_file_name(format!(
                ".compiler-{}-{}.tmp",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    pending = Some((Temporary(path), file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(fail(e)),
            }
        }
        let (temp, mut file) = pending.ok_or("cannot allocate unique output temporary file")?;
        file.write_all(bytes).map_err(fail)?;
        file.sync_all().map_err(fail)?;
        drop(file);
        self.check()?;
        atomic_move(&temp.0, &self.path, self.overwrite)
    }
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[cfg(windows)]
fn atomic_move(from: &Path, to: &Path, overwrite: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let flags = 8 | u32::from(overwrite);
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(fail(std::io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(not(windows))]
fn atomic_move(_from: &Path, _to: &Path, _overwrite: bool) -> Result<(), String> {
    Err("native publication requires a Windows host".into())
}
