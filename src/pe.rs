//! Dynamic Windows x64 PE builder. Production path uses no external linker.
use crate::x64::{MachineFragment, Symbol};
use std::collections::{BTreeMap, HashMap};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
    ZeroFill,
}
#[derive(Debug, Clone)]
pub struct DataSection {
    pub name: String,
    pub bytes: Vec<u8>,
    pub virtual_size: u32,
    pub access: Access,
    pub symbols: Vec<(Symbol, u32)>,
}
#[derive(Debug, Clone)]
pub struct Import {
    pub symbol: Symbol,
    pub dll: String,
    pub name: String,
}
#[derive(Debug, Clone)]
pub struct PointerFixup {
    pub section: usize,
    pub offset: u32,
    pub target: Symbol,
    pub addend: i64,
}
#[derive(Debug, Clone, Copy)]
pub struct DirectoryRef {
    pub section: usize,
    pub offset: u32,
    pub size: u32,
}
#[derive(Debug, Clone)]
pub struct ImageSpec {
    pub text: MachineFragment,
    pub code_symbols: Vec<(Symbol, u32)>,
    pub data: Vec<DataSection>,
    pub imports: Vec<Import>,
    pub pointers: Vec<PointerFixup>,
    pub entry_offset: u32,
    pub image_base: u64,
    pub exception_directory: Option<DirectoryRef>,
}
#[derive(Debug, Clone)]
pub struct SectionReport {
    pub name: String,
    pub rva: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}
#[derive(Debug, Clone)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub sections: Vec<SectionReport>,
    pub symbols: HashMap<Symbol, u32>,
    pub entry_rva: u32,
}
const LIMIT: u32 = 64 * 1024 * 1024;
fn align(n: u32, a: u32) -> Result<u32, String> {
    n.checked_add(a - 1)
        .map(|v| v & !(a - 1))
        .ok_or("PE alignment overflow".into())
}
fn size(n: usize) -> Result<u32, String> {
    u32::try_from(n).map_err(|_| "PE size exceeds u32".into())
}
fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn define(map: &mut HashMap<Symbol, u32>, s: Symbol, rva: u32) -> Result<(), String> {
    if map.insert(s, rva).is_some() {
        return Err(format!("duplicate native symbol {}", s.0));
    }
    Ok(())
}
fn ascii_name(s: &str) -> bool {
    !s.is_empty() && s.is_ascii() && !s.contains('\0')
}
#[derive(Default)]
struct ImportData {
    bytes: Vec<u8>,
    aliases: Vec<(Symbol, u32)>,
    rva_fields: Vec<usize>,
    lookup_entries: Vec<usize>,
    directory_size: u32,
    iat_offset: u32,
    iat_size: u32,
}
impl ImportData {
    fn create(imports: &[Import]) -> Result<Self, String> {
        if imports.is_empty() {
            return Ok(Self::default());
        }
        let mut groups: BTreeMap<String, BTreeMap<String, Vec<Symbol>>> = BTreeMap::new();
        for import in imports {
            if !ascii_name(&import.dll)
                || !ascii_name(&import.name)
                || import.dll.contains(['/', '\\'])
            {
                return Err("invalid import name".into());
            }
            groups
                .entry(import.dll.to_ascii_lowercase())
                .or_default()
                .entry(import.name.clone())
                .or_default()
                .push(import.symbol);
        }
        let descriptors = (groups.len() + 1)
            .checked_mul(20)
            .ok_or("import size overflow")?;
        let mut out = Self {
            bytes: vec![0; descriptors],
            directory_size: size(descriptors)?,
            ..Self::default()
        };
        out.bytes
            .resize(align(size(out.bytes.len())?, 8)? as usize, 0);
        let mut tables = Vec::new();
        for names in groups.values() {
            let at = size(out.bytes.len())?;
            let count = (names.len() + 1)
                .checked_mul(8)
                .ok_or("import size overflow")?;
            out.bytes.resize(
                out.bytes
                    .len()
                    .checked_add(count)
                    .ok_or("import size overflow")?,
                0,
            );
            tables.push((at, 0));
        }
        out.iat_offset = size(out.bytes.len())?;
        for (i, names) in groups.values().enumerate() {
            let at = size(out.bytes.len())?;
            let count = (names.len() + 1) * 8;
            out.bytes.resize(
                out.bytes
                    .len()
                    .checked_add(count)
                    .ok_or("import size overflow")?,
                0,
            );
            tables[i].1 = at;
        }
        out.iat_size = size(out.bytes.len())? - out.iat_offset;
        for (i, (dll, names)) in groups.iter().enumerate() {
            let dll_at = size(out.bytes.len())?;
            out.bytes.extend(dll.as_bytes());
            out.bytes.push(0);
            let (ilt, iat) = tables[i];
            put32(&mut out.bytes, i * 20, ilt);
            put32(&mut out.bytes, i * 20 + 12, dll_at);
            put32(&mut out.bytes, i * 20 + 16, iat);
            out.rva_fields.extend([i * 20, i * 20 + 12, i * 20 + 16]);
            for (j, (name, aliases)) in names.iter().enumerate() {
                if !out.bytes.len().is_multiple_of(2) {
                    out.bytes.push(0);
                }
                let at = size(out.bytes.len())?;
                out.bytes.extend([0, 0]);
                out.bytes.extend(name.as_bytes());
                out.bytes.push(0);
                for start in [ilt, iat] {
                    let offset = start as usize + j * 8;
                    put64(&mut out.bytes, offset, at as u64);
                    out.lookup_entries.push(offset);
                }
                for symbol in aliases {
                    out.aliases.push((*symbol, iat + j as u32 * 8));
                }
            }
        }
        Ok(out)
    }
    fn relocate(&mut self, rva: u32) -> Result<(), String> {
        for at in &self.rva_fields {
            let old = u32::from_le_bytes(self.bytes[*at..*at + 4].try_into().unwrap());
            put32(
                &mut self.bytes,
                *at,
                old.checked_add(rva).ok_or("import RVA overflow")?,
            );
        }
        for at in &self.lookup_entries {
            let old = u64::from_le_bytes(self.bytes[*at..*at + 8].try_into().unwrap());
            put64(
                &mut self.bytes,
                *at,
                old.checked_add(rva as u64)
                    .ok_or("import address overflow")?,
            );
        }
        Ok(())
    }
}
fn add_section(
    reports: &mut Vec<SectionReport>,
    name: String,
    virtual_size: u32,
    data_size: u32,
    flags: u32,
    next_rva: &mut u32,
    next_raw: &mut u32,
) -> Result<(), String> {
    let raw = if data_size == 0 {
        0
    } else {
        align(data_size, 512)?
    };
    let virtual_end = next_rva
        .checked_add(virtual_size)
        .ok_or("section RVA overflow")?;
    let raw_end = next_raw.checked_add(raw).ok_or("section file overflow")?;
    if virtual_end > LIMIT || raw_end > LIMIT {
        return Err("initial native image limit is 64 MiB".into());
    }
    reports.push(SectionReport {
        name,
        rva: *next_rva,
        virtual_size,
        raw_offset: if raw == 0 { 0 } else { *next_raw },
        raw_size: raw,
        characteristics: flags,
    });
    *next_rva = align(virtual_end, 4096)?;
    *next_raw = raw_end;
    Ok(())
}
/// Resolve owned machine fragments, data and imports into a native executable.
pub fn build(spec: &ImageSpec) -> Result<Image, String> {
    if spec.imports.len() > 65535 {
        return Err("too many native imports".into());
    }
    let import_name_bytes = spec.imports.iter().try_fold(0usize, |n, i| {
        n.checked_add(i.dll.len())?.checked_add(i.name.len())
    });
    if import_name_bytes.is_none_or(|n| n > LIMIT as usize / 2) {
        return Err("import names exceed native image budget".into());
    }

    let text_size = size(spec.text.bytes.len())?;
    if text_size == 0 || spec.entry_offset >= text_size {
        return Err("entrypoint outside text".into());
    }
    if !spec.image_base.is_multiple_of(65536) || spec.image_base == 0 {
        return Err("image base must be nonzero and 64-KiB aligned".into());
    }
    let count = 1 + spec.data.len() + usize::from(!spec.imports.is_empty()) + 1;
    if count > 96 {
        return Err("too many PE sections".into());
    }
    let headers = align(
        392 + size(count.checked_mul(40).ok_or("section table overflow")?)?,
        512,
    )?;
    let mut next_raw = headers;
    let mut next_rva = align(headers, 4096)?;
    let mut reports = Vec::new();
    add_section(
        &mut reports,
        ".text".into(),
        text_size,
        text_size,
        0x60000020,
        &mut next_rva,
        &mut next_raw,
    )?;
    let mut payloads = vec![spec.text.bytes.clone()];
    let mut symbols = HashMap::new();
    for (symbol, offset) in &spec.code_symbols {
        if *offset > text_size {
            return Err("code symbol outside text".into());
        }
        define(
            &mut symbols,
            *symbol,
            reports[0]
                .rva
                .checked_add(*offset)
                .ok_or("code RVA overflow")?,
        )?;
    }
    let mut used = std::collections::HashSet::from([
        ".text".to_string(),
        ".idata".to_string(),
        ".reloc".to_string(),
    ]);
    for data in &spec.data {
        if !ascii_name(&data.name) || data.name.len() > 8 || !used.insert(data.name.clone()) {
            return Err("invalid or duplicate PE section name".into());
        }
        let length = size(data.bytes.len())?;
        if data.virtual_size == 0 || length > data.virtual_size {
            return Err("invalid data virtual size".into());
        }
        if data.access == Access::ZeroFill && !data.bytes.is_empty() {
            return Err("zero-fill section cannot contain raw bytes".into());
        }
        let flags = match data.access {
            Access::ReadOnly => 0x40000040,
            Access::ReadWrite => 0xc0000040,
            Access::ZeroFill => 0xc0000080,
        };
        add_section(
            &mut reports,
            data.name.clone(),
            data.virtual_size,
            length,
            flags,
            &mut next_rva,
            &mut next_raw,
        )?;
        let section = reports.last().unwrap();
        for (symbol, offset) in &data.symbols {
            if *offset > data.virtual_size {
                return Err("data symbol outside section".into());
            }
            define(
                &mut symbols,
                *symbol,
                section
                    .rva
                    .checked_add(*offset)
                    .ok_or("data RVA overflow")?,
            )?;
        }
        payloads.push(data.bytes.clone());
    }
    let mut imports = ImportData::create(&spec.imports)?;
    let import_index = if imports.bytes.is_empty() {
        None
    } else {
        let index = reports.len();
        let len = size(imports.bytes.len())?;
        add_section(
            &mut reports,
            ".idata".into(),
            len,
            len,
            0xc0000040,
            &mut next_rva,
            &mut next_raw,
        )?;
        let rva = reports[index].rva;
        imports.relocate(rva)?;
        for (symbol, offset) in &imports.aliases {
            define(
                &mut symbols,
                *symbol,
                rva.checked_add(*offset).ok_or("import RVA overflow")?,
            )?;
        }
        payloads.push(imports.bytes.clone());
        Some(index)
    };
    let mut pages: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
    let mut pointer_locations = std::collections::HashSet::new();
    for fix in &spec.pointers {
        let section = spec
            .data
            .get(fix.section)
            .ok_or("pointer section index out of range")?;
        let end = fix.offset.checked_add(8).ok_or("pointer offset overflow")?;
        if end > size(section.bytes.len())? || fix.offset % 8 != 0 {
            return Err("pointer fixup must be aligned within raw data".into());
        }
        if !pointer_locations.insert((fix.section, fix.offset)) {
            return Err("duplicate pointer fixup".into());
        }
        let target = *symbols
            .get(&fix.target)
            .ok_or("unresolved pointer symbol")?;
        let value = i128::from(spec.image_base) + i128::from(target) + i128::from(fix.addend);
        let value = u64::try_from(value).map_err(|_| "absolute address overflow")?;
        put64(&mut payloads[fix.section + 1], fix.offset as usize, value);
        let at = reports[fix.section + 1]
            .rva
            .checked_add(fix.offset)
            .ok_or("relocation RVA overflow")?;
        pages
            .entry(at & !4095)
            .or_default()
            .push(0xa000 | ((at & 4095) as u16));
    }
    // All other references are RIP-relative. A padding-only block describes an
    // image that needs no address edits but is still movable by the loader.
    if pages.is_empty() {
        pages.insert(reports[0].rva, Vec::new());
    }
    let mut reloc = Vec::new();
    for (page, mut entries) in pages {
        entries.sort_unstable();
        if entries.is_empty() {
            entries.extend([0, 0]);
        } else if entries.len() % 2 != 0 {
            entries.push(0);
        }
        reloc.extend(page.to_le_bytes());
        reloc.extend((8u32 + entries.len() as u32 * 2).to_le_bytes());
        for entry in entries {
            reloc.extend(entry.to_le_bytes());
        }
    }
    let reloc_index = reports.len();
    let reloc_len = size(reloc.len())?;
    add_section(
        &mut reports,
        ".reloc".into(),
        reloc_len,
        reloc_len,
        0x42000040,
        &mut next_rva,
        &mut next_raw,
    )?;
    payloads.push(reloc);
    let exception = if let Some(dir) = spec.exception_directory {
        let data = spec
            .data
            .get(dir.section)
            .ok_or("exception section index out of range")?;
        if data.access != Access::ReadOnly
            || dir.size == 0
            || dir.size % 12 != 0
            || dir
                .offset
                .checked_add(dir.size)
                .is_none_or(|end| end > data.bytes.len() as u32)
        {
            return Err("invalid exception directory range".into());
        }
        Some((
            reports[dir.section + 1]
                .rva
                .checked_add(dir.offset)
                .ok_or("exception RVA overflow")?,
            dir.size,
        ))
    } else {
        None
    };
    spec.image_base
        .checked_add(next_rva as u64)
        .ok_or("image address overflow")?;
    payloads[0] = spec.text.resolve(reports[0].rva, &symbols)?;
    let mut out = vec![0; next_raw as usize];
    out[..2].copy_from_slice(b"MZ");
    put32(&mut out, 60, 128);
    out[128..132].copy_from_slice(b"PE\0\0");
    put16(&mut out, 132, 0x8664);
    put16(&mut out, 134, count as u16);
    put16(&mut out, 148, 240);
    put16(&mut out, 150, 0x22);
    let o = 152;
    put16(&mut out, o, 0x20b);
    put32(&mut out, o + 4, reports[0].raw_size);
    let initialized = reports[1..]
        .iter()
        .filter(|s| s.raw_size > 0)
        .try_fold(0u32, |a, s| a.checked_add(s.raw_size))
        .ok_or("data size overflow")?;
    let uninitialized = reports
        .iter()
        .filter(|s| s.characteristics & 0x80 != 0)
        .try_fold(0u32, |a, s| a.checked_add(s.virtual_size))
        .ok_or("bss size overflow")?;
    put32(&mut out, o + 8, initialized);
    put32(&mut out, o + 12, uninitialized);
    let entry = reports[0].rva + spec.entry_offset;
    put32(&mut out, o + 16, entry);
    put32(&mut out, o + 20, reports[0].rva);
    put64(&mut out, o + 24, spec.image_base);
    put32(&mut out, o + 32, 4096);
    put32(&mut out, o + 36, 512);
    put16(&mut out, o + 40, 6);
    put16(&mut out, o + 48, 6);
    put32(&mut out, o + 56, next_rva);
    put32(&mut out, o + 60, headers);
    put16(&mut out, o + 68, 3);
    put16(&mut out, o + 70, 0x160);
    for (off, value) in [(72, 1048576), (80, 4096), (88, 1048576), (96, 4096)] {
        put64(&mut out, o + off, value);
    }
    put32(&mut out, o + 108, 16);
    if let Some(index) = import_index {
        let rva = reports[index].rva;
        put32(&mut out, o + 120, rva);
        put32(&mut out, o + 124, imports.directory_size);
        put32(&mut out, o + 208, rva + imports.iat_offset);
        put32(&mut out, o + 212, imports.iat_size);
    }
    put32(&mut out, o + 152, reports[reloc_index].rva);
    put32(&mut out, o + 156, reloc_len);
    if let Some((rva, len)) = exception {
        put32(&mut out, o + 136, rva);
        put32(&mut out, o + 140, len);
    }
    for (i, (section, payload)) in reports.iter().zip(payloads).enumerate() {
        let at = 392 + i * 40;
        out[at..at + section.name.len()].copy_from_slice(section.name.as_bytes());
        put32(&mut out, at + 8, section.virtual_size);
        put32(&mut out, at + 12, section.rva);
        put32(&mut out, at + 16, section.raw_size);
        put32(&mut out, at + 20, section.raw_offset);
        put32(&mut out, at + 36, section.characteristics);
        if !payload.is_empty() {
            let start = section.raw_offset as usize;
            out[start..start + payload.len()].copy_from_slice(&payload);
        }
    }
    validate(&out)?;
    Ok(Image {
        bytes: out,
        sections: reports,
        symbols,
        entry_rva: entry,
    })
}
/// Validate the subset of unbound PE32+ images emitted by this builder. It is not
/// a general parser for third-party executables or already loader-bound images.
pub fn validate(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > LIMIT as usize {
        return Err("image exceeds validation budget".into());
    }
    fn read16(b: &[u8], n: usize) -> Result<u16, String> {
        let end = n.checked_add(2).ok_or("header offset overflow")?;
        Ok(u16::from_le_bytes(
            b.get(n..end).ok_or("truncated PE")?.try_into().unwrap(),
        ))
    }
    fn read32(b: &[u8], n: usize) -> Result<u32, String> {
        let end = n.checked_add(4).ok_or("header offset overflow")?;
        Ok(u32::from_le_bytes(
            b.get(n..end).ok_or("truncated PE")?.try_into().unwrap(),
        ))
    }
    fn range<'a>(
        bytes: &'a [u8],
        sections: &[SectionReport],
        rva: u32,
        len: u32,
    ) -> Result<&'a [u8], String> {
        let end = rva.checked_add(len).ok_or("directory overflow")?;
        for s in sections {
            let section_end = s
                .rva
                .checked_add(s.virtual_size.min(s.raw_size))
                .ok_or("section overflow")?;
            if rva >= s.rva && end <= section_end {
                let offset = s.raw_offset as usize + (rva - s.rva) as usize;
                return bytes
                    .get(offset..offset + len as usize)
                    .ok_or("directory outside file".into());
            }
        }
        Err("directory outside initialized section".into())
    }
    if bytes.get(..2) != Some(b"MZ") {
        return Err("invalid DOS signature".into());
    }
    let pe = read32(bytes, 60)? as usize;
    let coff = pe.checked_add(4).ok_or("header offset overflow")?;
    if bytes.get(pe..coff) != Some(b"PE\0\0") || read16(bytes, coff)? != 0x8664 {
        return Err("invalid PE signature/machine".into());
    }
    let count = read16(bytes, coff + 2)? as usize;
    if count == 0 || count > 96 || read16(bytes, coff + 16)? != 240 {
        return Err("invalid section count/optional size".into());
    }
    let o = coff + 20;
    if read16(bytes, o)? != 0x20b
        || read32(bytes, o + 32)? != 4096
        || read32(bytes, o + 36)? != 512
        || read32(bytes, o + 108)? != 16
    {
        return Err("unsupported PE layout".into());
    }
    let headers = read32(bytes, o + 60)?;
    let image_size = read32(bytes, o + 56)?;
    if headers % 512 != 0
        || image_size % 4096 != 0
        || headers as usize > bytes.len()
        || headers > LIMIT
        || image_size > LIMIT
    {
        return Err("invalid image/header size".into());
    }
    let table = o + 240;
    if table
        .checked_add(count * 40)
        .is_none_or(|end| end > headers as usize)
    {
        return Err("section table outside headers".into());
    }
    let mut sections = Vec::new();
    let mut names = std::collections::HashSet::new();
    let mut previous_virtual = align(headers, 4096)?;
    let mut previous_raw = headers;
    for i in 0..count {
        let at = table + i * 40;
        let name_bytes = bytes.get(at..at + 8).ok_or("truncated section")?;
        let end = name_bytes.iter().position(|b| *b == 0).unwrap_or(8);
        let name = std::str::from_utf8(&name_bytes[..end])
            .map_err(|_| "invalid section name")?
            .to_string();
        if name.is_empty() || !name.is_ascii() || !names.insert(name.clone()) {
            return Err("invalid section name".into());
        }
        let s = SectionReport {
            name,
            rva: read32(bytes, at + 12)?,
            virtual_size: read32(bytes, at + 8)?,
            raw_size: read32(bytes, at + 16)?,
            raw_offset: read32(bytes, at + 20)?,
            characteristics: read32(bytes, at + 36)?,
        };
        let end = s
            .rva
            .checked_add(s.virtual_size)
            .ok_or("section address overflow")?;
        if s.virtual_size == 0
            || !s.rva.is_multiple_of(4096)
            || s.rva < previous_virtual
            || end > image_size
        {
            return Err("invalid virtual section range".into());
        }
        if s.characteristics & 0xa0000000 == 0xa0000000 {
            return Err("writable executable section is forbidden".into());
        }
        previous_virtual = align(end, 4096)?;
        if s.raw_size == 0 {
            if s.raw_offset != 0 {
                return Err("raw-empty section offset must be zero".into());
            }
        } else {
            let raw_end = s
                .raw_offset
                .checked_add(s.raw_size)
                .ok_or("raw section overflow")?;
            if !s.raw_offset.is_multiple_of(512)
                || !s.raw_size.is_multiple_of(512)
                || s.raw_offset < previous_raw
                || raw_end as usize > bytes.len()
            {
                return Err("invalid raw section range".into());
            }
            previous_raw = raw_end;
        }
        sections.push(s);
    }
    let entry = read32(bytes, o + 16)?;
    if !sections.iter().any(|s| {
        entry >= s.rva && entry < s.rva + s.virtual_size && s.characteristics & 0x20000000 != 0
    }) {
        return Err("entrypoint outside executable section".into());
    }
    for i in 0..16 {
        let rva = read32(bytes, o + 112 + i * 8)?;
        let len = read32(bytes, o + 116 + i * 8)?;
        if rva == 0 && len == 0 {
            continue;
        }
        if rva == 0 || len == 0 {
            return Err("partial data directory".into());
        }
        let data = range(bytes, &sections, rva, len)?;
        match i {
            1 => {
                if len % 20 != 0 || len < 20 || !data[data.len() - 20..].iter().all(|b| *b == 0) {
                    return Err("invalid import descriptors".into());
                }
                for d in data[..data.len() - 20].chunks_exact(20) {
                    let ilt = read32(d, 0)?;
                    let name = read32(d, 12)?;
                    let iat = read32(d, 16)?;
                    if !terminated_name(bytes, &sections, name) {
                        return Err("invalid DLL name".into());
                    }
                    let mut offset = 0u32;
                    loop {
                        let lookup = range(
                            bytes,
                            &sections,
                            ilt.checked_add(offset).ok_or("import offset overflow")?,
                            8,
                        )?;
                        let address = range(
                            bytes,
                            &sections,
                            iat.checked_add(offset).ok_or("import offset overflow")?,
                            8,
                        )?;
                        if lookup != address {
                            return Err("unbound import tables differ".into());
                        }
                        let value = u64::from_le_bytes(lookup.try_into().unwrap());
                        if value == 0 {
                            break;
                        }
                        let hint =
                            u32::try_from(value).map_err(|_| "unsupported ordinal/large import")?;
                        range(bytes, &sections, hint, 2)?;
                        if !terminated_name(
                            bytes,
                            &sections,
                            hint.checked_add(2).ok_or("hint offset overflow")?,
                        ) {
                            return Err("invalid imported function name".into());
                        }
                        offset = offset.checked_add(8).ok_or("import table overflow")?;
                    }
                }
            }
            3 => {
                if len % 12 != 0 {
                    return Err("invalid exception directory size".into());
                }
            }
            5 => {
                let mut at = 0usize;
                while at < data.len() {
                    let page = read32(data, at)?;
                    let size = read32(data, at + 4)? as usize;
                    let end = at.checked_add(size).ok_or("relocation block overflow")?;
                    if page % 4096 != 0 || size < 8 || !size.is_multiple_of(4) || end > data.len() {
                        return Err("invalid relocation block".into());
                    }
                    for item in data[at + 8..end].chunks_exact(2) {
                        let v = u16::from_le_bytes(item.try_into().unwrap());
                        match v >> 12 {
                            0 => (),
                            10 => {
                                range(
                                    bytes,
                                    &sections,
                                    page.checked_add((v & 4095) as u32)
                                        .ok_or("relocation overflow")?,
                                    8,
                                )?;
                            }
                            _ => return Err("unsupported base relocation".into()),
                        }
                    }
                    at = end;
                }
            }
            12 => {
                if len % 8 != 0 {
                    return Err("invalid IAT size".into());
                }
            }
            _ => return Err("unsupported data directory in owned image".into()),
        }
    }
    Ok(())
}
fn terminated_name(bytes: &[u8], sections: &[SectionReport], rva: u32) -> bool {
    for s in sections {
        if rva >= s.rva {
            let offset = (rva - s.rva) as usize;
            let length = s.virtual_size.min(s.raw_size) as usize;
            if offset < length {
                let start = s.raw_offset as usize + offset;
                return bytes
                    .get(start..s.raw_offset as usize + length)
                    .is_some_and(|v| {
                        v.iter()
                            .position(|b| *b == 0)
                            .is_some_and(|end| end > 0 && v[..end].is_ascii())
                    });
            }
        }
    }
    false
}
