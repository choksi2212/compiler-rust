use compiler::context::{CompilationContext, SourceId, Stage};
use compiler::pipeline::NativeCompileOptions;
fn compile(name: &str, source: &str, optimize: bool) -> compiler::pipeline::NativeCompilation {
    let mut c = CompilationContext::default();
    let id = c.add_source(name, source);
    c.compile_native(
        id,
        &NativeCompileOptions {
            optimize,
            ..NativeCompileOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("{}", c.render(&e)))
}
#[test]
fn malformed_and_unsupported_source_is_diagnostic_not_an_image() {
    for (source, stage) in [
        ("@", Stage::Lex),
        ("int main(){int x=;return 0;}", Stage::Parse),
        ("int main(){x=2;return 0;}", Stage::Semantic),
        ("int main(){unknown();return 0;}", Stage::Semantic),
        ("int main(){int x=1;x++;return 0;}", Stage::Lex),
        (
            "int main(){printf(\"%d\",\"s\");return 0;}",
            Stage::Semantic,
        ),
    ] {
        let mut context = CompilationContext::default();
        let id = context.add_source("bad source.c", source);
        let e = context
            .compile_native(id, &NativeCompileOptions::default())
            .unwrap_err();
        assert_eq!(e.source, id);
        assert_eq!(e.stage, stage);
        assert!(context.render(&e).starts_with("bad source.c:"));
    }
    let context = CompilationContext::default();
    assert_eq!(
        context
            .compile_native(SourceId(99), &NativeCompileOptions::default())
            .unwrap_err()
            .stage,
        Stage::Input
    );
    let mut context = CompilationContext::default();
    let id = context.add_source("resource.c", "int main(){return 0;}");
    let mut options = NativeCompileOptions::default();
    options.runtime.output_capacity = 0;
    assert_eq!(
        context.compile_native(id, &options).unwrap_err().stage,
        Stage::Native
    );
}
#[cfg(all(windows, target_arch = "x86_64"))]
mod windows {
    use super::*;
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static ID: AtomicUsize = AtomicUsize::new(0);
    fn execute(
        compiled: &compiler::pipeline::NativeCompilation,
        source: &str,
    ) -> (i32, Vec<u8>, Vec<u8>) {
        let root = std::env::temp_dir().join(format!(
            "C source qualification Ω {} {}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let source_path = root.join("source program.c");
        std::fs::write(&source_path, source).unwrap();
        let executable = root.join("native program.exe");
        std::fs::write(&executable, &compiled.image.bytes).unwrap();
        std::fs::remove_file(source_path).unwrap();
        let run = root.join("fresh execution");
        std::fs::create_dir(&run).unwrap();
        let mut child = Command::new(&executable)
            .current_dir(&run)
            .env("PATH", "")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
        let stdout = std::thread::spawn(move || {
            let mut bytes = vec![];
            out.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let stderr = std::thread::spawn(move || {
            let mut bytes = vec![];
            err.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let start = std::time::Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed().as_secs() >= 5 {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("qualified source program exceeded5seconds");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let out = stdout.join().unwrap();
        let err = stderr.join().unwrap();
        std::fs::remove_file(executable).unwrap();
        std::fs::remove_dir(run).unwrap();
        std::fs::remove_dir(root).unwrap();
        (status.code().unwrap(), out, err)
    }
    #[test]
    fn all_six_sources_match_expected_and_both_vm_modes() {
        for name in [
            "addition",
            "loop",
            "factorial",
            "pattern",
            "conditional",
            "strings",
        ] {
            let path = format!("examples/{name}.c");
            let source = std::fs::read_to_string(&path).unwrap();
            let expected = std::fs::read_to_string(format!("examples/{name}.expected"))
                .unwrap()
                .replace("\r\n", "\n");
            for optimize in [false, true] {
                let compiled = compile(&path, &source, optimize);
                assert!(!compiled.tokens.is_empty());
                compiler::verify::verify(&compiled.ir).unwrap();
                compiler::verify::verify(&compiled.optimized_ir).unwrap();
                for ir in [&compiled.ir, &compiled.optimized_ir] {
                    let vm = compiler::bytecode::run(&compiler::bytecode::generate(ir).unwrap())
                        .unwrap();
                    assert_eq!(vm, expected, "{name} VM");
                }
                let (status, out, err) = execute(&compiled, &source);
                assert_eq!(
                    (status, out, err),
                    (0, expected.as_bytes().to_vec(), vec![]),
                    "{name} native optimize{optimize}"
                );
            }
        }
    }
    #[test]
    fn source_interactions_and_checked_failures() {
        assert_eq!("世界".len(), 6, "UTF-8 fixture corruption");
        let cases = [
            (
                "int main(){int x=0&&(1/0);int y=1||(1/0);printf(\"%d %d\\n\",x,y);return 0;}",
                0,
                "0 1\n",
                "",
            ),
            (
                "int main(){int x=4;{int x=8;printf(\"%d \",x);}printf(\"%d\",x);return 0;}",
                0,
                "8 4",
                "",
            ),
            (
                "int main(){int n=printf(\"%d\",0);printf(\"%d\",n);return 0;}",
                0,
                "01",
                "",
            ),
            (
                "int main(){printf(\"%s %d\",\"世界\",strlen(\"世界\"));return 0;}",
                0,
                "世界 6",
                "",
            ),
            (
                "int main(){printf(\"discarded\");int x=1/0;return 0;}",
                1,
                "",
                "division by zero",
            ),
            (
                "int main(){int x=2147483647+1;return 0;}",
                2,
                "",
                "overflow",
            ),
            (
                "int main(){int x=1;{int x=x;}return 0;}",
                3,
                "",
                "uninitialized",
            ),
            (
                "int main(){for(int i=0;i<2;i=i+1){int x;if(i==0){x=1;}else{printf(\"%d\",x);}}return 0;}",
                3,
                "",
                "uninitialized",
            ),
        ];
        for (source, status, output, error) in cases {
            for optimize in [false, true] {
                let result = execute(&compile("interactions.c", source, optimize), source);
                assert_eq!(result.0, status);
                assert_eq!(result.1, output.as_bytes());
                let err = String::from_utf8(result.2).unwrap();
                if error.is_empty() {
                    assert_eq!(err, "");
                } else {
                    assert!(err.contains(error), "{err}");
                }
            }
        }
    }
    #[test]
    fn large_output_and_native_execution_without_vm_budget() {
        let source = "int main(){for(int i=0;i<20000;i=i+1){printf(\"x\");}return 0;}";
        for opt in [false, true] {
            let (status, out, err) = execute(&compile("large.c", source, opt), source);
            assert_eq!(status, 0);
            assert_eq!(out, vec![b'x'; 20000]);
            assert!(err.is_empty());
        }
        let source = "int main(){int i=0;while(i<600000){i=i+1;}printf(\"%d\",i);return 0;}";
        let baseline = compile("budget.c", source, false);
        let vm = compiler::bytecode::run(&compiler::bytecode::generate(&baseline.ir).unwrap())
            .unwrap_err();
        assert!(vm.contains("instruction limit"));
        for opt in [false, true] {
            let (status, out, err) = execute(&compile("budget.c", source, opt), source);
            assert_eq!((status, out, err), (0, b"600000".to_vec(), vec![]));
        }
    }
}
#[test]
fn source_and_token_resource_bounds() {
    use compiler::lexer::*;
    assert!(lex_with_token_limit("a b", 2).is_ok());
    assert!(
        lex_with_token_limit("a b c", 2)
            .unwrap_err()
            .contains("1:5: token budget")
    );
    assert_eq!(lex_with_token_limit("", 0).unwrap().len(), 1);
    assert!(lex_with_token_limit("a", 0).is_err());
    let oversized = " ".repeat(MAX_SOURCE_BYTES + 1);
    assert!(lex(&oversized).unwrap_err().contains("16MiB"));
    let at_limit = " ".repeat(MAX_SOURCE_BYTES);
    assert_eq!(lex_with_token_limit(&at_limit, 0).unwrap().len(), 1);
}
