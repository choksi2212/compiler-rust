use compiler::{
    context::{CompilationContext, SourceId, Stage},
    pipeline::NativeCompileOptions,
    typed_lower,
};
fn compile(source: &str, optimize: bool) -> compiler::typed_pipeline::Compilation {
    let mut c = CompilationContext::for_semantics(compiler::context::Semantics::C11);
    let id = c.add_source("typed source.c", source);
    c.compile_typed(
        id,
        &NativeCompileOptions {
            optimize,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{}\n{source}", c.render(&e)))
}
#[cfg(all(windows, target_arch = "x86_64"))]
fn run(bytes: &[u8]) -> (i32, Vec<u8>, String) {
    use std::{
        fs,
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "typed source Ω {} {}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("program.exe");
    fs::write(&path, bytes).unwrap();
    let mut child = Command::new(path)
        .current_dir(&root)
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
            panic!("typed source native timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let out = child.wait_with_output().unwrap();
    fs::remove_dir_all(root).unwrap();
    (
        out.status.code().unwrap(),
        out.stdout,
        String::from_utf8(out.stderr).unwrap(),
    )
}
fn qualifies(source: &str, expected: i32) {
    for optimize in [false, true] {
        let built = compile(source, optimize);
        assert!(built.image.bytes.starts_with(b"MZ"));
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(
            run(&built.image.bytes),
            (expected, vec![], String::new()),
            "{source} optimize={optimize}"
        );
    }
}
#[test]
fn actual_native_literal_rank_conversions_and_widths() {
    for source in [
        "int main(){return (-1 < 4294967295) ? 0:1;}",
        "int main(){return (-1 < 0xffffffff) ? 1:0;}",
        "int main(){unsigned int u=4294967295u;long s=-1;unsigned long r=u+s;return r==4294967294UL && sizeof(u+s)==4 ? 0:1;}",
        "int main(){unsigned int u=4294967295u;long long s=-1;return u+s==4294967294LL && sizeof(u+s)==8 ? 0:1;}",
        "int main(){unsigned long long x=18446744073709551615ULL;return x/2==9223372036854775807ULL && x%2==1 ? 0:1;}",
        "int main(){unsigned long long x=18446744073709551615ULL;x++;return x==0 ? 0:1;}",
        "int main(){unsigned long long x=1;x<<=63;x>>=62;return x==2 ? 0:1;}",
        "int main(){long long x=-9223372036854775807LL-1;return (x>>63)==-1 ? 0:1;}",
        "int main(){signed char x=127;int old=x++;unsigned short s=65535;s++;_Bool b=0;--b;return old==127&&x==-128&&s==0&&b==1 ? 0:1;}",
        "int main(){char x=(char)255;unsigned char u=(unsigned char)-1;return x==-1&&u==255 ? 0:1;}",
        "int main(){_Bool b=256;_Bool c=0;return b==1&&c==0 ? 0:1;}",
        "int main(){unsigned long long x=1ULL<<32;_Bool b=x;if(!b)return 1;return x?0:2;}",
    ] {
        qualifies(source, 0);
    }
}

#[test]
fn documented_typed_demo_is_a_real_native_program() {
    qualifies(include_str!("../examples/typed_integers.c"), 0);
}
#[test]
fn typedef_declarators_layout_and_unevaluated_sizeof() {
    qualifies(
        "typedef int M[3][4];typedef int (*F)(int);int main(void){int x=0;return sizeof(M)==48 && sizeof(F)==8 && sizeof(long)==4 && sizeof(long long)==8 && sizeof(void*)==8 && _Alignof(short)==2 && sizeof(x++)==4 && x==0 ? 0:1;}",
        0,
    );
    qualifies(
        "typedef unsigned long long T;int main(){T x=9;{int T=2;T++;if(T!=3)return 1;}T y=3;T:if(x!=9||y!=3)return 2;goto end;end:return (int)sizeof(T);}",
        8,
    );
    qualifies(
        "int main(){return sizeof(\"ab\\0cd\")==6 && sizeof(\"世界\")==7 ? 0:1;}",
        0,
    );
    qualifies(
        "int main(){return (1 ? 2u : -1)==2u && sizeof(1?2u:-1)==4 ? 0:1;}",
        0,
    );
    qualifies(
        "int main(){int x=0;int y=(x=2,x+3);return y==5&&x==2 ? 0:1;}",
        0,
    );
}
#[test]
fn typed_control_flow_preserves_width_and_scope_lifetimes() {
    qualifies(
        "int main(){unsigned long long sum=0;for(unsigned long long i=0;i<20;i++){if(i==2)continue;if(i==10)break;sum+=i;}return sum==43 ? 0:1;}",
        0,
    );
    qualifies(
        "int main(){unsigned long long n=18446744073709551615ULL;int x=0;switch(n){case 0:x=2;break;case 18446744073709551615ULL:x=7;break;default:x=9;}return x;}",
        7,
    );
    qualifies(
        "int main(){int i=0;int s=0;do{i++;switch(i){case 2:continue;case 3:s+=10;break;default:s++;}}while(i<4);return s;}",
        12,
    );
    qualifies(
        "int main(){int x=0;0&&x++;1||++x;if(x)return 1;unsigned long long v=0;v=(1?9ULL:(unsigned long long)(1/0));return v==9 ? 0:1;}",
        0,
    );
    qualifies(
        "int main(){int n=0;again:n++;if(n<5)goto again;goto inner;{int x=99;inner:x=7;n+=x;goto end;}end:return n;}",
        12,
    );
    qualifies(
        "int main(){volatile unsigned long long x=0;while(x<7)x++;return (int)x;}",
        7,
    );
    qualifies("int main(){}", 0);
}
#[test]
fn compiler_modes_and_incomplete_features_fail_explicitly() {
    let mut c = CompilationContext::for_semantics(compiler::context::Semantics::C11);
    for (source, stage) in [
        ("#include <stdio.h>\nint main(){}", Stage::Lex),
        ("int main(){return 1.0;}", Stage::Parse),
        ("int main(){int a[3];return sizeof(a);}", Stage::Ir),
        ("int main(){int *p;return 0;}", Stage::Ir),
        ("int main(){printf(\"x\");}", Stage::Parse),
        ("typedef int T;", Stage::Ir),
    ] {
        let id = c.add_source("unsupported.c", source);
        let error = c.compile_typed(id, &Default::default()).unwrap_err();
        assert_eq!(error.stage, stage, "{source}");
    }
    let id = c.add_source(
        "type-only.c",
        "typedef int (*F)(int);int main(){int a[3];return sizeof(a);}",
    );
    assert!(c.check_typed(id).is_ok());
    assert!(c.compile_typed(SourceId(999), &Default::default()).is_err());
    let frontend = c.check_typed(id).unwrap();
    assert!(typed_lower::lower(&frontend.unit).is_err());
    let mut wrong = CompilationContext::default();
    let id = wrong.add_source("mode.c", "int main(){return 0;}");
    assert_eq!(
        wrong
            .compile_typed(id, &Default::default())
            .unwrap_err()
            .stage,
        Stage::Input
    );
    wrong.semantics = compiler::context::Semantics::C11;
    assert_eq!(
        wrong
            .compile_native(id, &Default::default())
            .unwrap_err()
            .stage,
        Stage::Input
    );
}
#[test]
fn runtime_faults_have_status_and_never_masquerade_as_successful_returns() {
    for (source, status, diagnostic) in [
        ("int main(){int x=2147483647;x++;return x;}", 2, "overflow"),
        (
            "int main(){long long x=9223372036854775807LL;return (int)(x+1);}",
            2,
            "overflow",
        ),
        (
            "int main(){unsigned long long x=1;return (int)(x/0);}",
            1,
            "division by zero",
        ),
        (
            "int main(){unsigned long long x=1;return (int)(x<<64);}",
            8,
            "invalid shift",
        ),
        ("int main(){int x;return x;}", 3, "uninitialized"),
        (
            "int main(){int n=0;again:if(n==1)goto inner;{int x=7;inner:if(n==1)return x;}n++;goto again;}",
            3,
            "uninitialized",
        ),
    ] {
        for optimize in [false, true] {
            let built = compile(source, optimize);
            #[cfg(all(windows, target_arch = "x86_64"))]
            {
                let (actual, stdout, stderr) = run(&built.image.bytes);
                assert_eq!(actual, status, "{source}");
                assert!(stdout.is_empty());
                assert!(stderr.contains(diagnostic));
            }
        }
    }
    for result in [1, 2, 3, 7, 8, -1] {
        qualifies(&format!("int main(){{return {result};}}"), result);
    }
}
