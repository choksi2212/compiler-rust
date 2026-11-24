use compiler::context::{CompilationContext, Stage};
use compiler::{bytecode, ir, lexer, optimize, parser, semantic};
fn compile() -> Result<i32, (i32, String)> {
    let options = Options::parse().map_err(|e| (2, e))?;
    let Some(options) = options else {
        return Ok(0);
    };
    let file = &options.input;
    let (resolved, source) =
        compiler::files::read_source(file).map_err(|e| (1, format!("{}: {e}", file.display())))?;
    let target = options
        .output
        .as_ref()
        .map(|output| {
            compiler::files::OutputTarget::prepare(
                output,
                &[
                    resolved.clone(),
                    std::env::current_exe().map_err(|e| e.to_string())?,
                ],
                options.overwrite,
            )
        })
        .transpose()
        .map_err(|e| (1, e))?;
    let mut context = CompilationContext::for_semantics(if options.flag("--std=c11") {
        compiler::context::Semantics::C11
    } else {
        compiler::context::Semantics::Teaching
    });
    let source_id = context.add_source(file.to_string_lossy(), source);
    if options.flag("--std=c11") {
        if options.flag("-fsyntax-only") {
            let frontend = context
                .check_typed(source_id)
                .map_err(|e| (1, context.render(&e)))?;
            if options.flag("--tokens") {
                eprintln!("TOKENS\n{:#?}", frontend.tokens);
            }
            if options.flag("--ast") {
                eprintln!("TYPED AST\n{:#?}", frontend.unit);
            }
            return Ok(0);
        }
        let built = context
            .compile_typed(
                source_id,
                &compiler::pipeline::NativeCompileOptions {
                    optimize: options.optimize(),
                    ..Default::default()
                },
            )
            .map_err(|e| (1, context.render(&e)))?;
        for (flag, label, value) in [
            (
                "--tokens",
                "TOKENS",
                format!("{:#?}", built.frontend.tokens),
            ),
            ("--ast", "TYPED AST", format!("{:#?}", built.frontend.unit)),
            ("--ir", "TYPED IR", format!("{:#?}", built.ir)),
            (
                "--optimized-ir",
                "OPTIMIZED TYPED IR",
                format!("{:#?}", built.optimized_ir),
            ),
        ] {
            if options.flag(flag) {
                eprintln!("{label}\n{value}");
            }
        }
        if options.flag("--emit=machine") {
            eprint!("{}", built.machine_dump);
        }
        target
            .ok_or_else(|| (2, "typed native output requires -o".into()))?
            .publish(&built.image.bytes)
            .map_err(|e| (1, e))?;
        return Ok(0);
    }
    if let Some(target) = target {
        let built = context
            .compile_native(
                source_id,
                &compiler::pipeline::NativeCompileOptions {
                    optimize: options.optimize(),
                    ..Default::default()
                },
            )
            .map_err(|e| (1, context.render(&e)))?;
        for (flag, label, value) in [
            ("--tokens", "TOKENS", format!("{:#?}", built.tokens)),
            ("--ast", "AST", format!("{:#?}", built.ast)),
            ("--ir", "THREE-ADDRESS IR", format!("{:#?}", built.ir)),
            (
                "--optimized-ir",
                "OPTIMIZED IR",
                format!("{:#?}", built.optimized_ir),
            ),
        ] {
            if options.flag(flag) {
                eprintln!("{label}\n{value}");
            }
        }
        if options.flag("--emit=machine") {
            eprint!("{}", built.machine_dump);
        }
        target.publish(&built.image.bytes).map_err(|e| (1, e))?;
        return Ok(0);
    }
    let source = &context
        .source(source_id)
        .map_err(|e| (1, context.render(&e)))?
        .text;
    let flag = |s: &str| options.flag(s);
    let result = (|| {
        let tokens = context.run_stage(source_id, Stage::Lex, || lexer::lex(source))?;
        if flag("--tokens") {
            eprintln!("TOKENS\n{tokens:#?}");
        }
        let ast = context.run_stage(source_id, Stage::Parse, || parser::parse(tokens))?;
        if flag("--ast") {
            eprintln!("AST\n{ast:#?}");
        }
        context.run_stage(source_id, Stage::Semantic, || semantic::check(&ast))?;
        let ir = ir::lower(&ast);
        context.run_stage(source_id, Stage::Ir, || compiler::verify::verify(&ir))?;
        if flag("--ir") {
            eprintln!("THREE-ADDRESS IR\n{ir:#?}");
        }
        let optimized = optimize::optimize(&ir);
        context.run_stage(source_id, Stage::Optimize, || {
            compiler::verify::verify(&optimized)
        })?;
        if flag("--optimized-ir") {
            eprintln!("OPTIMIZED IR\n{optimized:#?}");
        }
        let code = context.run_stage(source_id, Stage::Bytecode, || {
            bytecode::generate(if options.optimize() { &optimized } else { &ir })
        })?;
        if flag("--bytecode") {
            for (i, op) in code.iter().enumerate() {
                eprintln!("{i:04} {op:?}");
            }
        }
        let output = context.run_stage(source_id, Stage::Runtime, || bytecode::execute(&code))?;
        print!("{}", output.output);
        Ok(output.exit_code)
    })();
    result.map_err(|e| (1, context.render(&e)))
}
fn main() {
    match compile() {
        Ok(0) => (),
        Ok(code) => std::process::exit(code),
        Err((code, message)) => {
            eprintln!("{message}");
            std::process::exit(code);
        }
    }
}

const HELP: &str = "Usage: compiler <source.c> [-o output.exe] [options]
Without -o: compile and execute on the teaching VM (--run-vm is explicit).
With -o: build an independent Windows x64 executable; do not execute it.
  --overwrite       Permit replacing an existing output (never source/compiler)
  --tokens --ast --ir --optimized-ir   Stage dumps on stderr
  --bytecode        VM instruction dump (VM only)
  --emit=machine    Linked native bytes and encoder instructions (requires -o)
  --no-opt / -O0    Disable IR optimization (-O1 enables verified basic passes)
  --std=c11         Partial typed native integer mode; requires -o
  -fsyntax-only     Type-check supported declarations with --std=c11; no -o
  --semantics=teaching  Explicit checked teaching mode (the default)
  --version --print-capabilities --help
  --                End options; following argument is the source filename
Exit codes: 0 success, 1 compilation/IO/runtime failure, 2 invalid usage.
Supports the documented teaching C subset; full C11 is not implemented.";

struct Options {
    input: std::path::PathBuf,
    output: Option<std::path::PathBuf>,
    overwrite: bool,
    flags: Vec<String>,
}
impl Options {
    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|s| s == name)
    }
    fn optimize(&self) -> bool {
        !self.flag("--no-opt") && !self.flag("-O0")
    }
    fn parse() -> Result<Option<Self>, String> {
        let mut args = std::env::args_os().skip(1).peekable();
        if args.peek().is_none() {
            println!("{HELP}");
            return Ok(None);
        }
        let mut input = None;
        let mut output = None;
        let mut flags = vec![];
        let mut terminal = false;
        let mut informational = None;
        for_arg(
            &mut args,
            &mut input,
            &mut output,
            &mut flags,
            &mut terminal,
            &mut informational,
        )?;
        if let Some(info) = informational {
            if input.is_some() || output.is_some() || !flags.is_empty() {
                return Err("informational options must be used alone".into());
            }
            match info.as_str() {
                "--help" | "-h" => println!("{HELP}"),
                "--version" => println!(
                    "compiler {} (teaching C subset, Windows x64 native)",
                    env!("CARGO_PKG_VERSION")
                ),
                _ => println!(
                    r#"{{"version":"{}","language":"teaching C subset","full_c11":false,"preprocessor":false,"target":"windows-x86_64","native_pe":true,"external_compiler_required":false,"source_limit_bytes":16777216,"token_limit":1000000,"integer_bits":32,"checked_arithmetic":true,"printf_formats":["%d","%s"],"native_output_default_bytes":16777216,"vm_default":true,"control_flow":["if","while","for","do","break","continue","switch","goto","integer-return"],"integer_updates":true,"comments":true,"integer_radices":[8,10,16],"typed_integer_suffixes":true,"typed_mode_requires":"--std=c11","typed_vm":false,"typed_integer_widths":[8,16,32,64],"typed_calls":true,"typed_globals":true,"typed_variadics":true,"typed_memory_execution":true,"typed_fixed_arrays":true,"typed_indirect_calls":true,"owned_allocator":true,"typed_records":true,"typed_enums":true,"typed_bitfields":true,"typed_aggregate_abi":true,"typed_designated_initializers":true,"typed_compound_literals":true,"typed_alignas_max":16,"typed_static_assert":true,"typed_generic":true}}"#,
                    env!("CARGO_PKG_VERSION")
                ),
            }
            return Ok(None);
        }
        let options = Self {
            input: input.ok_or("missing source file")?,
            output,
            overwrite: flags.iter().any(|s| s == "--overwrite"),
            flags,
        };
        if options.output.is_some() && (options.flag("--run-vm") || options.flag("--bytecode")) {
            return Err("-o conflicts with --run-vm and --bytecode".into());
        }
        if options.output.is_none() && (options.overwrite || options.flag("--emit=machine")) {
            return Err("--overwrite and --emit=machine require -o".into());
        }
        let typed = options.flag("--std=c11");
        let syntax = options.flag("-fsyntax-only");
        if options.flag("-O1") && (options.flag("-O0") || options.flag("--no-opt")) {
            return Err("conflicting optimization options".into());
        }
        if syntax
            && (!typed
                || options.output.is_some()
                || options.flag("--ir")
                || options.flag("--optimized-ir"))
        {
            return Err("-fsyntax-only requires --std=c11 and conflicts with -o/IR dumps".into());
        }
        if typed
            && (options.flag("--run-vm")
                || options.flag("--bytecode")
                || options.flag("--semantics=teaching"))
        {
            return Err("--std=c11 conflicts with teaching/VM modes".into());
        }
        if typed && options.output.is_none() && !syntax {
            return Err(
                "--std=c11 requires -o or -fsyntax-only; typed VM execution is unsupported".into(),
            );
        }
        Ok(Some(options))
    }
}
fn for_arg(
    args: &mut impl Iterator<Item = std::ffi::OsString>,
    input: &mut Option<std::path::PathBuf>,
    output: &mut Option<std::path::PathBuf>,
    flags: &mut Vec<String>,
    terminal: &mut bool,
    informational: &mut Option<String>,
) -> Result<(), String> {
    while let Some(arg) = args.next() {
        if !*terminal && arg == "--" {
            *terminal = true;
            continue;
        }
        if !*terminal && arg == "-o" {
            if output.is_some() {
                return Err("duplicate -o".into());
            }
            *output = Some(args.next().ok_or("-o requires an output path")?.into());
        } else if !*terminal && arg.to_string_lossy().starts_with('-') {
            let text = arg.to_str().ok_or("invalid option encoding")?;
            if ["--help", "-h", "--version", "--print-capabilities"].contains(&text) {
                if informational.replace(text.into()).is_some() {
                    return Err("use one informational option".into());
                }
            } else if [
                "--tokens",
                "--ast",
                "--ir",
                "--optimized-ir",
                "--bytecode",
                "--no-opt",
                "--run-vm",
                "--emit=machine",
                "--overwrite",
                "--std=c11",
                "--semantics=teaching",
                "-fsyntax-only",
                "-O0",
                "-O1",
            ]
            .contains(&text)
            {
                flags.push(text.into());
            } else {
                return Err(format!("unknown option {text}"));
            }
        } else if input.replace(arg.into()).is_some() {
            return Err("expected one input file".into());
        }
    }
    Ok(())
}
