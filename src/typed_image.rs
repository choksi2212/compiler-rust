//! Combine typed functions with the same owned startup/runtime/PE path as teaching mode.
use crate::{
    abi::{FramePlan, FrameRequest},
    native_integer::NativeFunction,
    native_runtime::{self, ModuleProgram, RuntimeOptions},
    typed_ir::{self, Instruction, Module},
    typed_native::{self, ModuleBindings},
    types::{IntegerKind, Type},
    x64::{Address, Binary, Condition, Instruction as M, Label, LabelAllocator, Reg, Width},
};
use std::collections::{BTreeMap, HashMap, HashSet};
fn entry(main: Label, labels: &mut LabelAllocator) -> Result<NativeFunction, String> {
    let rbx = Reg::new(3)?;
    let scratch = Reg::new(10)?;
    let frame = FramePlan::new(&FrameRequest {
        has_calls: true,
        saved: vec![rbx],
        ..Default::default()
    })?;
    let prologue = frame.prologue(labels.fresh()?)?;
    let mut code = prologue.instructions;
    let failed = labels.fresh()?;
    let exit = labels.fresh()?;
    code.extend([
        M::Mov {
            width: Width::Qword,
            dst: rbx,
            src: Reg::RCX,
        },
        M::Call(main),
        M::Load {
            width: Width::Dword,
            dst: scratch,
            address: Address::base(rbx, 24),
        },
        M::Binary {
            width: Width::Dword,
            op: Binary::Test,
            dst: scratch,
            src: scratch,
        },
        M::Branch {
            condition: Condition::NotEqual,
            label: failed,
        },
        M::Store {
            width: Width::Dword,
            address: Address::base(rbx, 28),
            src: Reg::RAX,
        },
        M::MovImm {
            width: Width::Dword,
            dst: Reg::RAX,
            value: 0,
        },
        M::Jump(exit),
        M::Bind(failed),
        M::Mov {
            width: Width::Dword,
            dst: Reg::RAX,
            src: scratch,
        },
        M::Bind(exit),
    ]);
    code.extend(frame.epilogue());
    Ok(NativeFunction {
        instructions: code,
        unwind: prologue.unwind,
        frame,
    })
}
pub fn build(
    module: &Module,
    options: &RuntimeOptions,
) -> Result<native_runtime::NativeBuild, String> {
    typed_ir::verify(module)?;
    let main = module
        .entry
        .and_then(|id| module.functions.iter().find(|f| f.id == id))
        .ok_or("native image requires an entry main function")?;
    let Type::Function {
        result,
        parameters,
        variadic,
        ..
    } = module.types.get(main.signature)?
    else {
        return Err("invalid main signature".into());
    };
    if *result != module.types.integer(IntegerKind::Int) || !parameters.is_empty() || *variadic {
        return Err("image entry requires int main(void)".into());
    }
    let mut strings = BTreeMap::new();
    for data in &module.data {
        if data.bytes.last() != Some(&0) {
            return Err("literal data requires a terminating NUL".into());
        }
        let text = String::from_utf8(data.bytes[..data.bytes.len() - 1].to_vec())
            .map_err(|_| "literal data is outside the delivered UTF-8 representation")?;
        strings.insert(data.id.0, text);
    }
    native_runtime::build_module(
        options,
        &strings,
        &BTreeMap::new(),
        |labels, runtime, context, symbols, memory| {
            let mut bindings = ModuleBindings {
                functions: HashMap::new(),
                function_addresses: HashMap::new(),
                literals: HashMap::new(),
                context,
                globals: HashMap::new(),
            };
            let mut ranges = HashMap::new();
            for function in &module.functions {
                let start = labels.fresh()?;
                let end = labels.fresh()?;
                bindings.functions.insert(function.symbol, start);
                bindings
                    .function_addresses
                    .insert(function.symbol, symbols.fresh()?);
                ranges.insert(function.id, (start, end));
            }
            for data in &module.data {
                bindings
                    .literals
                    .insert(data.id, runtime.constants[&data.id.0]);
            }

            let mut used: HashSet<_> = module
                .functions
                .iter()
                .flat_map(|f| &f.blocks)
                .flat_map(|b| &b.instructions)
                .filter_map(|i| {
                    if let Instruction::DirectCall { symbol, .. }
                    | Instruction::FunctionAddress { symbol, .. } = i
                    {
                        Some(*symbol)
                    } else {
                        None
                    }
                })
                .collect();
            let mut static_addresses = HashSet::new();
            for global in &module.globals {
                initializer_symbols(&global.initial, &mut static_addresses);
            }
            used.extend(static_addresses.iter().copied());
            let mut bridges = vec![];
            let mut allocation_labels: Option<(Label, Label)> = None;
            for external in &module.externs {
                if used.contains(&external.symbol) {
                    if external.name=="printf" && (module.functions.iter().flat_map(|f|&f.blocks).flat_map(|b|&b.instructions).any(|i|matches!(i,Instruction::FunctionAddress{symbol,..} if *symbol==external.symbol)) || static_addresses.contains(&external.symbol)) {
                        return Err("taking the owned printf address waits for general runtime-format support".into());
                    }
                    let print = crate::typed_runtime::service(
                        &module.types,
                        &external.name,
                        external.signature,
                        external.linkage,
                    )?;
                    use crate::typed_runtime::Service;
                    if matches!(
                        print,
                        Service::Malloc | Service::Free | Service::Calloc | Service::Realloc
                    ) && allocation_labels.is_none()
                    {
                        let allocate = labels.fresh()?;
                        let allocate_end = labels.fresh()?;
                        let release = labels.fresh()?;
                        let release_end = labels.fresh()?;
                        bridges.push((
                            allocate,
                            allocate_end,
                            crate::native_allocator::malloc(memory.allocate, labels)?,
                        ));
                        bridges.push((
                            release,
                            release_end,
                            crate::native_allocator::free(memory.free, labels)?,
                        ));
                        allocation_labels = Some((allocate, release));
                    }
                    let start = match (print, allocation_labels) {
                        (Service::Malloc, Some((allocate, _))) => allocate,
                        (Service::Free, Some((_, release))) => release,
                        _ => labels.fresh()?,
                    };
                    let end = labels.fresh()?;
                    bindings.functions.insert(external.symbol, start);
                    bindings
                        .function_addresses
                        .insert(external.symbol, symbols.fresh()?);
                    match (print, allocation_labels) {
                        (Service::Malloc | Service::Free, _) => {}
                        (Service::Calloc, Some((allocate, _))) => bridges.push((
                            start,
                            end,
                            crate::native_allocator::calloc(allocate, labels)?,
                        )),
                        (Service::Realloc, Some((allocate, release))) => bridges.push((
                            start,
                            end,
                            crate::native_allocator::realloc(allocate, release, labels)?,
                        )),
                        _ => bridges.push((
                            start,
                            end,
                            crate::typed_runtime::bridge(
                                print,
                                runtime.formatter,
                                context,
                                labels,
                            )?,
                        )),
                    }
                }
            }
            let (data, pointers) = global_data(module, symbols, &mut bindings)?;
            let code_symbols = bindings
                .function_addresses
                .iter()
                .map(|(id, symbol)| (*symbol, bindings.functions[id]))
                .collect();
            let mut functions: Vec<_> = typed_native::lower_functions(module, labels, &bindings)?
                .into_iter()
                .map(|(id, function)| {
                    let (start, end) = ranges[&id];
                    (start, end, function)
                })
                .collect();
            functions.extend(bridges);
            Ok(ModuleProgram {
                entry: entry(bindings.functions[&main.symbol], labels)?,
                functions,
                data,
                pointers,
                code_symbols,
            })
        },
    )
}

fn global_data(
    module: &Module,
    symbols: &mut crate::x64::SymbolAllocator,
    bindings: &mut ModuleBindings,
) -> Result<(Vec<crate::pe::DataSection>, Vec<crate::pe::PointerFixup>), String> {
    use crate::{
        pe::{Access, DataSection, PointerFixup},
        typed_ir::GlobalInitializer,
    };
    let mut groups = vec![
        DataSection {
            name: ".grdata".into(),
            bytes: vec![],
            virtual_size: 0,
            access: Access::ReadOnly,
            symbols: vec![],
        },
        DataSection {
            name: ".gdata".into(),
            bytes: vec![],
            virtual_size: 0,
            access: Access::ReadWrite,
            symbols: vec![],
        },
        DataSection {
            name: ".gbss".into(),
            bytes: vec![],
            virtual_size: 0,
            access: Access::ZeroFill,
            symbols: vec![],
        },
    ];
    for global in &module.globals {
        if global.defined {
            bindings
                .globals
                .insert(global.symbol, (symbols.fresh()?, global.ty));
        }
    }
    let mut pointers = vec![];
    for global in &module.globals {
        if !global.defined {
            continue;
        }
        let ty = module.types.unqualified(global.ty)?;
        if !matches!(
            module.types.get(ty)?,
            Type::Integer(_) | Type::Pointer(_) | Type::Array { .. }
        ) {
            return Err("non-scalar global storage waits for the memory phase".into());
        }
        let layout = module.types.layout(ty)?;
        let size = u32::try_from(layout.size).map_err(|_| "global layout too large")?;
        let group = if module.types.qualifiers(global.ty)?.constant {
            0
        } else if matches!(global.initial, GlobalInitializer::Zero) {
            2
        } else {
            1
        };
        let section = &mut groups[group];
        let align = layout.align;
        let offset = section
            .virtual_size
            .checked_add(align - 1)
            .map(|n| n / align * align)
            .ok_or("global alignment overflow")?;
        section.virtual_size = offset.checked_add(size).ok_or("global section overflow")?;
        section
            .symbols
            .push((bindings.globals[&global.symbol].0, offset));
        if group != 2 {
            section.bytes.resize(section.virtual_size as usize, 0);
        }
        let mut leaves = vec![];
        initial_leaves(
            &module.types,
            global.ty,
            &global.initial,
            offset,
            &mut leaves,
        )?;
        for (offset, size, initial) in leaves {
            match initial {
                GlobalInitializer::Array(_) => {
                    return Err("array initializer was not flattened".into());
                }
                GlobalInitializer::Zero => (),
                GlobalInitializer::Integer(bits) => section.bytes
                    [offset as usize..(offset + size) as usize]
                    .copy_from_slice(&bits.to_le_bytes()[..size as usize]),
                GlobalInitializer::Address { symbol, addend } => {
                    let target = bindings
                        .globals
                        .get(symbol)
                        .map(|g| g.0)
                        .or_else(|| bindings.function_addresses.get(symbol).copied())
                        .ok_or("unresolved initializer target")?;
                    pointers.push(PointerFixup {
                        section: group,
                        offset,
                        target,
                        addend: *addend,
                    });
                }
                GlobalInitializer::DataAddress { data, addend } => {
                    let target = *bindings
                        .literals
                        .get(data)
                        .ok_or("unresolved initializer literal")?;
                    pointers.push(PointerFixup {
                        section: group,
                        offset,
                        target,
                        addend: *addend,
                    });
                }
            }
        }
    }
    let mut data = vec![];
    let mut indexes = [None; 3];
    for (i, section) in groups.into_iter().enumerate() {
        if section.virtual_size > 0 {
            indexes[i] = Some(data.len());
            data.push(section);
        }
    }
    for pointer in &mut pointers {
        pointer.section = indexes[pointer.section].ok_or("initializer section disappeared")?;
    }
    Ok((data, pointers))
}

fn initial_leaves<'a>(
    types: &crate::types::TypeContext,
    ty: crate::types::TypeId,
    initial: &'a typed_ir::GlobalInitializer,
    offset: u32,
    leaves: &mut Vec<(u32, u32, &'a typed_ir::GlobalInitializer)>,
) -> Result<(), String> {
    if let typed_ir::GlobalInitializer::Array(values) = initial {
        let Type::Array { element, .. } = types.get(types.unqualified(ty)?)? else {
            return Err("array initializer storage mismatch".into());
        };
        let stride = u32::try_from(types.layout(*element)?.size)
            .map_err(|_| "array stride exceeds image budget")?;
        for (i, value) in values.iter().enumerate() {
            let at = offset
                .checked_add(
                    u32::try_from(i)
                        .map_err(|_| "array index overflow")?
                        .checked_mul(stride)
                        .ok_or("array offset overflow")?,
                )
                .ok_or("array layout overflow")?;
            initial_leaves(types, *element, value, at, leaves)?;
        }
    } else {
        leaves.push((
            offset,
            u32::try_from(types.layout(ty)?.size)
                .map_err(|_| "initializer layout exceeds image budget")?,
            initial,
        ));
    }
    Ok(())
}

fn initializer_symbols(
    initial: &typed_ir::GlobalInitializer,
    symbols: &mut HashSet<typed_ir::SymbolId>,
) {
    match initial {
        typed_ir::GlobalInitializer::Array(values) => {
            for value in values {
                initializer_symbols(value, symbols);
            }
        }
        typed_ir::GlobalInitializer::Address { symbol, .. } => {
            symbols.insert(*symbol);
        }
        _ => {}
    }
}
