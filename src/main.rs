use compiler::context::{CompilationContext, Stage};
use compiler::{bytecode, ir, lexer, optimize, parser, semantic};
fn compile() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "Usage: compiler <source.c> [--tokens] [--ast] [--ir] [--optimized-ir] [--bytecode] [--no-opt]\nCompiles the documented C subset to stack bytecode and executes it.\nStage dumps go to stderr; program output goes to stdout."
        );
        return Ok(());
    }
    let mut file = None;
    for a in &args {
        if a.starts_with('-') {
            if ![
                "--tokens",
                "--ast",
                "--ir",
                "--optimized-ir",
                "--bytecode",
                "--no-opt",
            ]
            .contains(&a.as_str())
            {
                return Err(format!("unknown option {a}"));
            }
        } else if file.replace(a).is_some() {
            return Err("expected one input file".into());
        }
    }
    let file = file.ok_or("missing source file")?;
    let source = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let mut context = CompilationContext::default();
    let source_id = context.add_source(file, source);
    let source = &context
        .source(source_id)
        .map_err(|e| context.render(&e))?
        .text;
    let flag = |s: &str| args.iter().any(|a| a == s);
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
            bytecode::generate(if flag("--no-opt") { &ir } else { &optimized })
        })?;
        if flag("--bytecode") {
            for (i, op) in code.iter().enumerate() {
                eprintln!("{i:04} {op:?}");
            }
        }
        let output = context.run_stage(source_id, Stage::Runtime, || bytecode::run(&code))?;
        print!("{output}");
        Ok(())
    })();
    result.map_err(|e| context.render(&e))
}
fn main() {
    if let Err(e) = compile() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
