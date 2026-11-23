mod bytecode;
mod ir;
mod lexer;
mod optimize;
mod parser;
mod semantic;
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
    let flag = |s: &str| args.iter().any(|a| a == s);
    let result = (|| {
        let tokens = lexer::lex(&source)?;
        if flag("--tokens") {
            eprintln!("TOKENS\n{tokens:#?}");
        }
        let ast = parser::parse(tokens)?;
        if flag("--ast") {
            eprintln!("AST\n{ast:#?}");
        }
        semantic::check(&ast)?;
        let ir = ir::lower(&ast);
        if flag("--ir") {
            eprintln!("THREE-ADDRESS IR\n{ir:#?}");
        }
        let optimized = optimize::optimize(&ir);
        if flag("--optimized-ir") {
            eprintln!("OPTIMIZED IR\n{optimized:#?}");
        }
        let code = bytecode::generate(if flag("--no-opt") { &ir } else { &optimized })?;
        if flag("--bytecode") {
            for (i, op) in code.iter().enumerate() {
                eprintln!("{i:04} {op:?}");
            }
        }
        let output = bytecode::run(&code)?;
        print!("{output}");
        Ok(())
    })();
    result.map_err(|e: String| format!("{file}:{e}"))
}
fn main() {
    if let Err(e) = compile() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod stress;
