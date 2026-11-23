//! Optional developer oracle fixture generator. Writes only owned target outputs.
use compiler::context::CompilationContext;
use compiler::pipeline::NativeCompileOptions;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::Path::new("target/P08-oracle");
    std::fs::create_dir_all(dir)?;
    let mut seed = 2212u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        seed >> 32
    };
    let ops = [
        "+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||",
    ];
    for case in 0..100 {
        let a = (next() % 101) as i32 - 50;
        let b = next() % 20 + 1;
        let bound = next() % 16;
        let op = ops[(next() % ops.len() as u64) as usize];
        let source = format!(
            "#include <stdio.h>\nint main(){{int a={a};int b={b};int sum=0;for(int i=0;i<{bound};i=i+1){{if(i%2==0){{sum=sum+i;}}else{{sum=sum-1;}}}}{{int a=7;printf(\"%d \",a);}}printf(\"%d %d %d\\n\",a{op}b,sum,!(a<b));return 0;}}\n"
        );
        let path = dir.join(format!("case{case:03}.c"));
        std::fs::write(&path, &source)?;
        for optimize in [false, true] {
            let mut context = CompilationContext::default();
            let id = context.add_source(path.display().to_string(), source.clone());
            let artifact = context
                .compile_native(
                    id,
                    &NativeCompileOptions {
                        optimize,
                        ..NativeCompileOptions::default()
                    },
                )
                .map_err(|e| context.render(&e))?;
            std::fs::write(
                dir.join(format!("case{case:03}-{optimize}.exe")),
                artifact.image.bytes,
            )?;
        }
    }
    println!("Generated100 defined-behavior C fixtures and200 native executables; seed2212");
    Ok(())
}
