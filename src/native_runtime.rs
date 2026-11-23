//! Project-owned native teaching runtime: formatting, byte output and startup.
use crate::abi::{FramePlan, FrameRequest, FunctionUnwind, with_unwind};
use crate::ir::{Ir, Value};
use crate::native_integer::{NativeFunction, RuntimeBindings, lower_with_runtime};
use crate::pe::{self, Access, DataSection, Image, ImageSpec, Import};
use crate::x64::*;
use std::collections::BTreeMap;
fn r(n: u8) -> Reg {
    Reg::new(n).unwrap()
}
fn small(dst: Reg, value: u32) -> Instruction {
    Instruction::MovImm {
        width: Width::Dword,
        dst,
        value: value as u64,
    }
}
fn mov(dst: Reg, src: Reg) -> Instruction {
    Instruction::Mov {
        width: Width::Qword,
        dst,
        src,
    }
}
fn load(dst: Reg, base: Reg, offset: i32) -> Instruction {
    Instruction::Load {
        width: Width::Qword,
        dst,
        address: Address::base(base, offset),
    }
}
fn store(base: Reg, offset: i32, src: Reg) -> Instruction {
    Instruction::Store {
        width: Width::Qword,
        address: Address::base(base, offset),
        src,
    }
}
fn test(reg: Reg) -> Instruction {
    Instruction::Binary {
        width: Width::Qword,
        op: Binary::Test,
        dst: reg,
        src: reg,
    }
}
fn compare(dst: Reg, src: Reg) -> Instruction {
    Instruction::Binary {
        width: Width::Qword,
        op: Binary::Compare,
        dst,
        src,
    }
}
fn change(reg: Reg, value: i32) -> Instruction {
    Instruction::Immediate {
        width: Width::Qword,
        op: if value >= 0 {
            Immediate::Add
        } else {
            Immediate::Sub
        },
        reg,
        value: value.abs(),
    }
}
fn branch(condition: Condition, label: Label) -> Instruction {
    Instruction::Branch { condition, label }
}
fn append_byte(code: &mut Vec<Instruction>, full: Label) {
    code.extend([
        compare(r(14), r(8)),
        branch(Condition::AboveEqual, full),
        Instruction::StoreByte {
            address: Address {
                base: r(15),
                index: Some((r(14), Scale::One)),
                displacement: 0,
            },
            src: Reg::RAX,
        },
        change(r(14), 1),
    ]);
}
/// Native formatter ABI: (context, format, argument_words, argument_count).
pub fn formatter(labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        locals: vec![(8, 8), (32, 8)],
        saved: [3, 6, 7, 12, 13, 14, 15].iter().map(|n| r(*n)).collect(),
        ..FrameRequest::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let again = labels.fresh()?;
    let literal = labels.fresh()?;
    let integer = labels.fresh()?;
    let string = labels.fresh()?;
    let negative = labels.fresh()?;
    let digits = labels.fresh()?;
    let digit_loop = labels.fresh()?;
    let reverse = labels.fresh()?;
    let string_loop = labels.fresh()?;
    let done = labels.fresh()?;
    let full = labels.fresh()?;
    let invalid = labels.fresh()?;
    let exit = labels.fresh()?;
    let error = labels.fresh()?;
    let count = frame.local_offsets()[0] as i32;
    let digit_start = frame.local_offsets()[1] as i32;
    code.extend([
        mov(r(3), Reg::RCX),
        mov(r(6), Reg::RDX),
        mov(r(7), r(8)),
        store(Reg::RSP, count, r(9)),
        small(r(12), 0),
        load(r(13), r(3), 8),
        mov(r(14), r(13)),
        load(r(15), r(3), 0),
        load(r(8), r(3), 16),
        Instruction::Bind(again),
        Instruction::LoadByte {
            dst: Reg::RAX,
            address: Address::base(r(6), 0),
        },
        test(Reg::RAX),
        branch(Condition::Equal, done),
        change(r(6), 1),
        Instruction::Immediate {
            width: Width::Dword,
            op: Immediate::Compare,
            reg: Reg::RAX,
            value: 37,
        },
        branch(Condition::NotEqual, literal),
        load(r(9), Reg::RSP, count),
        compare(r(12), r(9)),
        branch(Condition::AboveEqual, invalid),
        Instruction::LoadByte {
            dst: Reg::RAX,
            address: Address::base(r(6), 0),
        },
        change(r(6), 1),
        Instruction::Immediate {
            width: Width::Dword,
            op: Immediate::Compare,
            reg: Reg::RAX,
            value: 100,
        },
        branch(Condition::Equal, integer),
        Instruction::Immediate {
            width: Width::Dword,
            op: Immediate::Compare,
            reg: Reg::RAX,
            value: 115,
        },
        branch(Condition::Equal, string),
        Instruction::Jump(invalid),
        Instruction::Bind(literal),
    ]);
    append_byte(&mut code, full);
    code.extend([
        Instruction::Jump(again),
        Instruction::Bind(integer),
        Instruction::Load {
            width: Width::Dword,
            dst: Reg::RAX,
            address: Address {
                base: r(7),
                index: Some((r(12), Scale::Eight)),
                displacement: 0,
            },
        },
        change(r(12), 1),
        Instruction::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: Reg::RAX,
            src: Reg::RAX,
        },
        branch(Condition::Sign, negative),
        Instruction::Jump(digits),
        Instruction::Bind(negative),
        Instruction::Negate {
            width: Width::Dword,
            reg: Reg::RAX,
        },
        mov(r(10), Reg::RAX),
        small(Reg::RAX, 45),
    ]);
    append_byte(&mut code, full);
    code.extend([
        mov(Reg::RAX, r(10)),
        Instruction::Bind(digits),
        small(r(9), 0),
        small(r(11), 10),
        Instruction::Bind(digit_loop),
        Instruction::SignExtend(Width::Qword),
        Instruction::Divide {
            width: Width::Qword,
            divisor: r(11),
        },
        change(Reg::RDX, 48),
        Instruction::StoreByte {
            address: Address {
                base: Reg::RSP,
                index: Some((r(9), Scale::One)),
                displacement: digit_start,
            },
            src: Reg::RDX,
        },
        change(r(9), 1),
        test(Reg::RAX),
        branch(Condition::NotEqual, digit_loop),
        Instruction::Bind(reverse),
        change(r(9), -1),
        Instruction::LoadByte {
            dst: Reg::RAX,
            address: Address {
                base: Reg::RSP,
                index: Some((r(9), Scale::One)),
                displacement: digit_start,
            },
        },
    ]);
    append_byte(&mut code, full);
    code.extend([
        test(r(9)),
        branch(Condition::NotEqual, reverse),
        Instruction::Jump(again),
        Instruction::Bind(string),
        Instruction::Load {
            width: Width::Qword,
            dst: r(10),
            address: Address {
                base: r(7),
                index: Some((r(12), Scale::Eight)),
                displacement: 0,
            },
        },
        change(r(12), 1),
        Instruction::Bind(string_loop),
        Instruction::LoadByte {
            dst: Reg::RAX,
            address: Address::base(r(10), 0),
        },
        test(Reg::RAX),
        branch(Condition::Equal, again),
    ]);
    append_byte(&mut code, full);
    code.extend([
        change(r(10), 1),
        Instruction::Jump(string_loop),
        Instruction::Bind(done),
        load(r(9), Reg::RSP, count),
        compare(r(12), r(9)),
        branch(Condition::NotEqual, invalid),
        store(r(3), 8, r(14)),
        mov(Reg::RAX, r(14)),
        Instruction::Binary {
            width: Width::Qword,
            op: Binary::Sub,
            dst: Reg::RAX,
            src: r(13),
        },
        Instruction::Jump(exit),
        Instruction::Bind(full),
        small(Reg::RAX, 4),
        Instruction::Jump(error),
        Instruction::Bind(invalid),
        small(Reg::RAX, 5),
        Instruction::Bind(error),
        Instruction::Store {
            width: Width::Dword,
            address: Address::base(r(3), 24),
            src: Reg::RAX,
        },
        small(Reg::RAX, u32::MAX),
        Instruction::Bind(exit),
    ]);
    code.extend(frame.epilogue());
    encode(&code)?;
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
/// Native synchronous WriteAll ABI: (handle, bytes, length) -> status0/7.
pub fn write_all(
    labels: &mut LabelAllocator,
    write_file: Symbol,
) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        max_call_arguments: 5,
        locals: vec![(8, 8)],
        saved: vec![r(3), r(6), r(7)],
        ..FrameRequest::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let again = labels.fresh()?;
    let count_ready = labels.fresh()?;
    let success = labels.fresh()?;
    let failure = labels.fresh()?;
    let exit = labels.fresh()?;
    let written = frame.local_offsets()[0] as i32;
    code.extend([
        mov(r(3), Reg::RCX),
        mov(r(6), Reg::RDX),
        mov(r(7), r(8)),
        small(Reg::RAX, u32::MAX),
        compare(r(7), Reg::RAX),
        branch(Condition::Above, failure),
        Instruction::Bind(again),
        test(r(7)),
        branch(Condition::Equal, success),
        mov(Reg::RCX, r(3)),
        mov(Reg::RDX, r(6)),
        mov(r(8), r(7)),
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Compare,
            reg: r(8),
            value: 4096,
        },
        branch(Condition::BelowEqual, count_ready),
        small(r(8), 4096),
        Instruction::Bind(count_ready),
        Instruction::Lea {
            dst: r(9),
            address: Address::base(Reg::RSP, written),
        },
        small(Reg::RAX, 0),
        store(Reg::RSP, 32, Reg::RAX),
        Instruction::CallImport(write_file),
        Instruction::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: Reg::RAX,
            src: Reg::RAX,
        },
        branch(Condition::Equal, failure),
        Instruction::Load {
            width: Width::Dword,
            dst: Reg::RAX,
            address: Address::base(Reg::RSP, written),
        },
        test(Reg::RAX),
        branch(Condition::Equal, failure),
        compare(Reg::RAX, r(7)),
        branch(Condition::Above, failure),
        Instruction::Immediate {
            width: Width::Qword,
            op: Immediate::Compare,
            reg: Reg::RAX,
            value: 4096,
        },
        branch(Condition::Above, failure),
        Instruction::Binary {
            width: Width::Qword,
            op: Binary::Add,
            dst: r(6),
            src: Reg::RAX,
        },
        Instruction::Binary {
            width: Width::Qword,
            op: Binary::Sub,
            dst: r(7),
            src: Reg::RAX,
        },
        Instruction::Jump(again),
        Instruction::Bind(failure),
        small(Reg::RAX, 7),
        Instruction::Jump(exit),
        Instruction::Bind(success),
        small(Reg::RAX, 0),
        Instruction::Bind(exit),
    ]);
    code.extend(frame.epilogue());
    encode(&code)?;
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub output_capacity: u32,
}
impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            output_capacity: 16 * 1024 * 1024,
        }
    }
}
struct Pool {
    bytes: Vec<u8>,
    symbols: Vec<(Symbol, u32)>,
    cache: BTreeMap<String, Symbol>,
    allocator: SymbolAllocator,
}
impl Pool {
    fn add(&mut self, text: &str) -> Result<Symbol, String> {
        if let Some(s) = self.cache.get(text) {
            return Ok(*s);
        }
        let end = self
            .bytes
            .len()
            .checked_add(text.len())
            .and_then(|n| n.checked_add(1))
            .ok_or("literal data overflow")?;
        if end > 32 * 1024 * 1024 {
            return Err("native literal budget is32MiB".into());
        }
        let symbol = self.allocator.fresh()?;
        self.symbols.push((symbol, self.bytes.len() as u32));
        self.bytes.extend(text.as_bytes());
        self.bytes.push(0);
        self.cache.insert(text.into(), symbol);
        Ok(symbol)
    }
}
#[derive(Debug, Clone, Copy)]
struct Imports {
    allocate: Symbol,
    free: Symbol,
    handle: Symbol,
    write: Symbol,
    exit: Symbol,
}
fn startup(
    labels: &mut LabelAllocator,
    imports: Imports,
    context: Symbol,
    program: Label,
    writer: Label,
    messages: &[(u32, Symbol, u32)],
    capacity: u32,
) -> Result<NativeFunction, String> {
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        max_call_arguments: 4,
        saved: vec![r(3), r(12), r(13), r(14)],
        ..FrameRequest::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let alloc_failure = labels.fresh()?;
    let dispatch = labels.fresh()?;
    let emit_error = labels.fresh()?;
    let cleanup = labels.fresh()?;
    let quit = labels.fresh()?;
    let free_ok = labels.fresh()?;
    let output_ok = labels.fresh()?;
    code.extend([
        Instruction::LeaSymbol {
            dst: r(3),
            symbol: context,
        },
        small(Reg::RCX, 0),
        small(Reg::RDX, capacity),
        small(r(8), 0x3000),
        small(r(9), 4),
        Instruction::CallImport(imports.allocate),
        test(Reg::RAX),
        branch(Condition::Equal, alloc_failure),
        store(r(3), 0, Reg::RAX),
        mov(Reg::RCX, r(3)),
        Instruction::Call(program),
        mov(r(14), Reg::RAX),
        test(Reg::RAX),
        branch(Condition::NotEqual, dispatch),
        small(Reg::RCX, (-11i32) as u32),
        Instruction::CallImport(imports.handle),
        mov(Reg::RCX, Reg::RAX),
        load(Reg::RDX, r(3), 0),
        load(r(8), r(3), 8),
        Instruction::Call(writer),
        test(Reg::RAX),
        branch(Condition::Equal, output_ok),
        small(r(14), 7),
        Instruction::Jump(dispatch),
        Instruction::Bind(output_ok),
        Instruction::Load {
            width: Width::Dword,
            dst: r(14),
            address: Address::base(r(3), 28),
        },
        Instruction::Jump(cleanup),
        Instruction::Bind(alloc_failure),
        small(r(14), 6),
        Instruction::Bind(dispatch),
    ]);
    for (status, symbol, len) in messages {
        let next = labels.fresh()?;
        code.extend([
            Instruction::Immediate {
                width: Width::Dword,
                op: Immediate::Compare,
                reg: r(14),
                value: *status as i32,
            },
            branch(Condition::NotEqual, next),
            Instruction::LeaSymbol {
                dst: r(12),
                symbol: *symbol,
            },
            small(r(13), *len),
            Instruction::Jump(emit_error),
            Instruction::Bind(next),
        ]);
    }
    let (_, fallback, len) = messages
        .iter()
        .find(|(s, _, _)| *s == 5)
        .ok_or("missing generic runtime error")?;
    code.extend([
        small(r(14), 5),
        Instruction::LeaSymbol {
            dst: r(12),
            symbol: *fallback,
        },
        small(r(13), *len),
        Instruction::Bind(emit_error),
        small(Reg::RCX, (-12i32) as u32),
        Instruction::CallImport(imports.handle),
        mov(Reg::RCX, Reg::RAX),
        mov(Reg::RDX, r(12)),
        mov(r(8), r(13)),
        Instruction::Call(writer),
        test(Reg::RAX),
        branch(Condition::Equal, cleanup),
        small(r(14), 7),
        Instruction::Bind(cleanup),
        load(Reg::RCX, r(3), 0),
        test(Reg::RCX),
        branch(Condition::Equal, quit),
        small(Reg::RDX, 0),
        small(r(8), 0x8000),
        Instruction::CallImport(imports.free),
        Instruction::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: Reg::RAX,
            src: Reg::RAX,
        },
        branch(Condition::NotEqual, free_ok),
        small(Reg::RAX, 0),
        store(r(3), 0, Reg::RAX),
        small(r(14), 6),
        Instruction::Jump(dispatch),
        Instruction::Bind(free_ok),
        Instruction::Bind(quit),
        mov(Reg::RCX, r(14)),
        Instruction::CallImport(imports.exit),
    ]);
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
/// Build a native image from verified legacy IR and the owned teaching runtime.
/// This API intentionally does not load source files or publish executables.
pub fn image(ir: &[Ir], options: &RuntimeOptions) -> Result<Image, String> {
    Ok(build(ir, options)?.image)
}

#[derive(Debug)]
pub struct NativeBuild {
    pub image: Image,
    /// Encoder instruction offsets with final linked image bytes.
    pub machine_dump: String,
}

pub fn build(ir: &[Ir], options: &RuntimeOptions) -> Result<NativeBuild, String> {
    crate::verify::verify(ir)?;
    let mut strings = BTreeMap::new();
    let mut formats = BTreeMap::new();
    for (index, op) in ir.iter().enumerate() {
        match op {
            Ir::Const(_, Value::Str(text)) => {
                strings.insert(index, text.clone());
            }
            Ir::Print(format, _, _) => {
                formats.insert(index, format.clone());
            }
            _ => (),
        }
    }
    build_program(options, &strings, &formats, |labels, bindings| {
        lower_with_runtime(ir, labels, bindings)
    })
}
/// Shared owned startup/runtime/PE construction for verified program lowerers.
pub fn build_program(
    options: &RuntimeOptions,
    strings: &BTreeMap<usize, String>,
    formats: &BTreeMap<usize, String>,
    lower: impl FnOnce(&mut LabelAllocator, &RuntimeBindings) -> Result<NativeFunction, String>,
) -> Result<NativeBuild, String> {
    if !(1..=64 * 1024 * 1024).contains(&options.output_capacity) {
        return Err("native output capacity must be1..64MiB".into());
    }
    let mut symbols = SymbolAllocator::default();
    let imports = Imports {
        allocate: symbols.fresh()?,
        free: symbols.fresh()?,
        handle: symbols.fresh()?,
        write: symbols.fresh()?,
        exit: symbols.fresh()?,
    };
    let mut pool = Pool {
        bytes: vec![],
        symbols: vec![],
        cache: BTreeMap::new(),
        allocator: symbols,
    };
    let mut labels = LabelAllocator::default();
    let root_start = labels.fresh()?;
    let root_end = labels.fresh()?;
    let program_start = labels.fresh()?;
    let program_end = labels.fresh()?;
    let format_start = labels.fresh()?;
    let format_end = labels.fresh()?;
    let write_start = labels.fresh()?;
    let write_end = labels.fresh()?;
    let mut bindings = RuntimeBindings {
        formatter: format_start,
        constants: BTreeMap::new(),
        formats: BTreeMap::new(),
    };
    for (index, text) in strings {
        bindings.constants.insert(*index, pool.add(text)?);
    }
    for (index, format) in formats {
        bindings.formats.insert(*index, pool.add(format)?);
    }
    let mut messages = vec![];
    for (status, message) in [
        (1, "runtime error: division by zero\n"),
        (2, "runtime error: integer overflow\n"),
        (3, "runtime error: read of uninitialized variable\n"),
        (4, "runtime error: output capacity exceeded\n"),
        (5, "runtime error: invalid runtime format\n"),
        (6, "runtime error: memory service failure\n"),
        (7, "runtime error: output IO failure\n"),
        (8, "runtime error: invalid shift\n"),
    ] {
        messages.push((status, pool.add(message)?, message.len() as u32));
    }
    let context = pool.allocator.fresh()?;
    let program = lower(&mut labels, &bindings)?;
    let formatting = formatter(&mut labels)?;
    let writing = write_all(&mut labels, imports.write)?;
    let root = startup(
        &mut labels,
        imports,
        context,
        program_start,
        write_start,
        &messages,
        options.output_capacity,
    )?;
    let mut code = vec![];
    let mut metadata = vec![];
    for (start, end, function) in [
        (root_start, root_end, root),
        (program_start, program_end, program),
        (format_start, format_end, formatting),
        (write_start, write_end, writing),
    ] {
        code.push(Instruction::Bind(start));
        code.extend(function.instructions);
        code.push(Instruction::Bind(end));
        metadata.push((start, end, function.unwind));
    }
    let text = encode(&code)?;
    let offsets: BTreeMap<usize, u32> = text
        .listing
        .iter()
        .filter_map(|row| match row.instruction {
            Instruction::Bind(label) => Some((label.0, row.offset as u32)),
            _ => None,
        })
        .collect();
    let functions: Vec<_> = metadata
        .into_iter()
        .map(|(start, end, unwind)| FunctionUnwind {
            begin_offset: offsets[&start.0],
            end_offset: offsets[&end.0],
            unwind,
        })
        .collect();
    let mut state = vec![0; 32];
    state[16..24].copy_from_slice(&(options.output_capacity as u64).to_le_bytes());
    let data = vec![
        DataSection {
            name: ".rdata".into(),
            virtual_size: pool.bytes.len() as u32,
            bytes: pool.bytes,
            access: Access::ReadOnly,
            symbols: pool.symbols,
        },
        DataSection {
            name: ".data".into(),
            virtual_size: 32,
            bytes: state,
            access: Access::ReadWrite,
            symbols: vec![(context, 0)],
        },
    ];
    let mut os = vec![];
    for (symbol, name) in [
        (imports.allocate, "VirtualAlloc"),
        (imports.free, "VirtualFree"),
        (imports.handle, "GetStdHandle"),
        (imports.write, "WriteFile"),
        (imports.exit, "ExitProcess"),
    ] {
        os.push(Import {
            symbol,
            dll: "kernel32.dll".into(),
            name: name.into(),
        });
    }
    let listing = text.listing.clone();
    let spec = ImageSpec {
        text,
        code_symbols: vec![],
        data,
        imports: os,
        pointers: vec![],
        entry_offset: 0,
        image_base: 0x140000000,
        exception_directory: None,
    };
    let spec = with_unwind(&spec, &functions)?;
    let image = pe::build(&spec)?;
    let section = image
        .sections
        .iter()
        .find(|s| s.name == ".text")
        .ok_or("missing text section")?;
    let mut machine_dump =
        String::from("MACHINE CODE (linked .text RVA, bytes, encoder instruction)\n");
    use std::fmt::Write;
    for row in listing {
        let start = section.raw_offset as usize + row.offset;
        let bytes = &image.bytes[start..start + row.length];
        let hex = bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(
            machine_dump,
            "{:08x} {hex:<32} {:?}",
            section.rva as usize + row.offset,
            row.instruction
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(NativeBuild {
        image,
        machine_dump,
    })
}
