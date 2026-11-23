use compiler::{
    bytecode, context::CompilationContext, lexer, parser, pipeline::NativeCompileOptions, semantic,
};

fn compile(source: &str, optimize: bool) -> compiler::pipeline::NativeCompilation {
    let mut context = CompilationContext::default();
    let id = context.add_source("control.c", source);
    context
        .compile_native(
            id,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}\n{source}", context.render(&e)))
}

#[cfg(all(windows, target_arch = "x86_64"))]
fn native(image: &[u8]) -> (i32, String, String) {
    use std::{
        fs,
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static ID: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "control native {} {}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("program.exe");
    fs::write(&path, image).unwrap();
    let mut child = Command::new(path)
        .env("PATH", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if start.elapsed().as_secs() >= 3 {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native control-flow timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let out = child.wait_with_output().unwrap();
    fs::remove_dir_all(root).unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn qualifies(source: &str, expected: &str, exit: i32) {
    for optimize in [false, true] {
        let built = compile(source, optimize);
        let ir = if optimize {
            &built.optimized_ir
        } else {
            &built.ir
        };
        let vm = bytecode::execute(&bytecode::generate(ir).unwrap()).unwrap();
        assert_eq!(
            (vm.output.as_str(), vm.exit_code),
            (expected, exit),
            "{source}"
        );
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(
            native(&built.image.bytes),
            (exit, expected.into(), String::new()),
            "{source}"
        );
    }
}

#[test]
fn updates_assignment_precedence_and_short_circuit() {
    qualifies(
        "int main(void){int a=5;int b=a++;int c=++a;int d=a--;int e=--a;a+=3;a-=1;a*=4;a/=2;a%=9;printf(\"%d %d %d %d %d\",a,b,c,d,e);}",
        "5 5 7 7 5",
        0,
    );
    qualifies(
        "int main(){int a=0;int b=0;a=b=4;int x=0;0&&x++;1||++x;printf(\"%d %d %d\",a,b,x);return +7;}",
        "4 4 0",
        7,
    );
    qualifies(
        "int main(){int x=2;int y=(x+=3)*2;printf(\"%d %d\",x,y);return y-5;}",
        "5 10",
        5,
    );
    qualifies(
        "int main(){int i=0;while(i++<3){}printf(\"%d\",i);return 0;}",
        "4",
        0,
    );
}

#[test]
fn loops_continue_break_and_empty_clauses() {
    qualifies(
        "int main(){int sum=0;for(int i=0;i<8;i++){if(i==2)continue;if(i==6)break;sum+=i;}printf(\"%d\",sum);}",
        "13",
        0,
    );
    qualifies(
        "int main(){int i=0;int s=0;for(;;){i++;if(i<3)continue;s+=i;if(i==5)break;}printf(\"%d\",s);}",
        "12",
        0,
    );
    qualifies(
        "int main(){int i=0;int s=0;do{i++;if(i==2)continue;s+=i;}while(i<4);printf(\"%d %d\",i,s);}",
        "4 8",
        0,
    );
    qualifies(
        "int main(){int i=0;do{i++;break;}while(1/0);while(0){return 99;}for(;0;)return 4;printf(\"%d\",i);}",
        "1",
        0,
    );
}

#[test]
fn switch_fallthrough_nested_dispatch_and_loop_continue() {
    qualifies(
        "int main(){int s=0;for(int i=0;i<5;i++){switch(i){case 0:s+=1;case 1:s+=10;break;case 2:continue;default:s+=100;}s+=1000;}printf(\"%d\",s);}",
        "4221",
        0,
    );
    qualifies(
        "int main(){switch(2){default:printf(\"bad\");break;case 1:printf(\"one\");break;case 1+1:printf(\"two\");switch(0){case 0:printf(\"inside\");break;default:;}printf(\"end\");}switch(7){case 1:printf(\"bad\");}return 0;}",
        "twoinsideend",
        0,
    );
    qualifies(
        "int main(){int x=3;switch(x++){case 3:printf(\"%d\",x);break;default:printf(\"bad\");}return 3;}",
        "4",
        3,
    );
    qualifies(
        "int main(){switch(2){case 0:case 1:printf(\"bad\");break;case 2:;int x=9;printf(\"%d\",x);}return 0;}",
        "9",
        0,
    );
}

#[test]
fn gotos_scope_entries_and_return_status_are_real() {
    qualifies(
        "int main(){int i=0;again:i++;if(i<4)goto again;goto done;printf(\"bad\");done:printf(\"%d\",i);return 27;}",
        "4",
        27,
    );
    qualifies(
        "int main(){int i=0;goto inside;{int x=999;inside:x=5;i+=x;goto out;}out:printf(\"%d\",i);return -1;}",
        "5",
        -1,
    );
    qualifies(
        "int main(){printf(\"before\");if(1)return 2;printf(\"bad\");return 0;}",
        "before",
        2,
    );
    qualifies("int main(){return -2147483647-1;}", "", i32::MIN);
    qualifies("int main(){}", "", 0);
}

#[test]
fn bases_character_escapes_nul_and_comments() {
    qualifies(
        "/* heading */int main(void){// line\nint n=010+0x10+10;printf(\"%d %d %d %d\",n,'A','\\101','\\x41');/* tail */}",
        "34 65 65 65",
        0,
    );
    qualifies(
        "int main(){printf(\"%d %s\",strlen(\"ab\\0ignored\"),\"x\\0ignored\");printf(\"done\\0%d\");printf(\"'\\\"\\\\\\?\");}",
        "2 xdone'\"\\?",
        0,
    );
    let tokens = lexer::lex("/*a\n b*/int // comment\n x=0x2aUL;").unwrap();
    assert_eq!((tokens[0].line, tokens[0].column), (2, 5));
    assert_eq!((tokens[1].line, tokens[1].column), (3, 2));
    assert!(
        matches!(&tokens[3].kind, lexer::Kind::Integer(n) if n.magnitude==42 && n.radix==16 && n.suffix=="UL")
    );
    for source in [
        "/*",
        "/*x*",
        "08",
        "0x",
        "0xg",
        "1lul",
        "1lL",
        "''",
        "'ab'",
        "'\\x'",
        "'\\400'",
        "'世界'",
        "\"\\x80\"",
    ] {
        assert!(lexer::lex(source).is_err(), "{source}");
    }
    for source in ["int main(){return 1U;}", "int main(){return 0xffffffff;}"] {
        assert!(
            parser::parse(lexer::lex(source).unwrap()).is_err(),
            "{source}"
        );
    }
}

#[test]
fn semantic_errors_do_not_become_executables() {
    for body in [
        "break;",
        "continue;",
        "case 0:;",
        "default:;",
        "goto missing;",
        "a:;a:;",
        "(1+2)++;",
        "1=2;",
        "int x=1;(+x)++;",
        "int x=1;(+x)=2;",
        "int x=0;int y=(x+1)+=2;",
        "switch(1){case 1:;case 1:;}",
        "switch(1){default:;default:;}",
        "int x=1;switch(x){case x:;}",
        "switch(1){case 1/0:;}",
        "return \"str\";",
    ] {
        let source = format!("int main(){{{body}}}");
        let result = lexer::lex(&source)
            .and_then(parser::parse)
            .and_then(|ast| semantic::check(&ast));
        assert!(result.is_err(), "{source}");
    }
}

#[test]
fn update_faults_and_jumps_cannot_reuse_a_previous_scope_lifetime() {
    for (body, status, diagnostic) in [
        ("int x=2147483647;x++;", 2, "overflow"),
        ("int x=-2147483647-1;--x;", 2, "overflow"),
        ("int x=1;x/=0;", 1, "division by zero"),
        ("int x;x++;", 3, "uninitialized"),
        (
            "int i=0;again:if(i==1)goto inside;{int x=9;inside:if(i==1)printf(\"%d\",x);}i++;if(i<2)goto again;",
            3,
            "uninitialized",
        ),
        (
            "for(int i=0;i<2;i++){switch(i){int x=9;case 0:x=8;break;case 1:printf(\"%d\",x);}}",
            3,
            "uninitialized",
        ),
    ] {
        let source = format!("int main(){{{body}return 0;}}");
        for optimize in [false, true] {
            let built = compile(&source, optimize);
            let code = bytecode::generate(if optimize {
                &built.optimized_ir
            } else {
                &built.ir
            })
            .unwrap();
            assert!(
                bytecode::execute(&code).unwrap_err().contains(diagnostic),
                "{source}"
            );
            #[cfg(all(windows, target_arch = "x86_64"))]
            {
                let (actual, stdout, stderr) = native(&built.image.bytes);
                assert_eq!(actual, status, "{source}");
                assert!(stdout.is_empty());
                assert!(stderr.contains(diagnostic));
            }
        }
    }
}

#[test]
fn new_control_syntax_mutations_and_depth_fail_without_panics() {
    let seed = "int main(){int i=0;do{switch(i++){case 1:continue;default:break;}if(i>4)goto out;}while(i<8);out:return i;}";
    for i in 0..seed.len() {
        for replacement in ["", ";", ":", "++", "case", "}"] {
            let mut source = seed.to_string();
            source.replace_range(i..i + 1, replacement);
            assert!(
                std::panic::catch_unwind(|| lexer::lex(&source).and_then(parser::parse).and_then(
                    |ast| {
                        semantic::check(&ast)?;
                        let ir = compiler::ir::lower(&ast);
                        compiler::verify::verify(&ir)?;
                        let optimized = compiler::optimize::optimize(&ir);
                        compiler::verify::verify(&optimized)?;
                        compiler::native_runtime::image(&optimized, &Default::default()).map(|_| ())
                    }
                ))
                .is_ok(),
                "{source}"
            );
        }
    }
    assert!(
        parser::parse(lexer::lex(&format!("int main(){{{}return 0;}}", "a:".repeat(200))).unwrap())
            .is_err()
    );
}
