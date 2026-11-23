#![cfg(all(windows, target_arch = "x86_64"))]
use compiler::{
    context::{CompilationContext, Semantics},
    pipeline::NativeCompileOptions,
    typed_front, typed_lower,
};
use std::{
    fs,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
static ID: AtomicUsize = AtomicUsize::new(0);
fn compile(source: &str, optimize: bool) -> compiler::typed_pipeline::Compilation {
    let mut context = CompilationContext::for_semantics(Semantics::C11);
    let id = context.add_source("functions.c", source);
    context
        .compile_typed(
            id,
            &NativeCompileOptions {
                optimize,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}\n{source}", context.render(&e)))
}
fn execute(bytes: &[u8]) -> (i32, String, String) {
    let root = std::env::temp_dir().join(format!(
        "C functions {} {}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
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
    let started = std::time::Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed().as_secs() >= 4 {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("C function native execution timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let output = child.wait_with_output().unwrap();
    fs::remove_dir_all(root).unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}
fn qualifies(source: &str, expected: i32) {
    for optimize in [false, true] {
        let built = compile(source, optimize);
        assert_eq!(
            execute(&built.image.bytes),
            (expected, String::new(), String::new()),
            "{source} opt={optimize}"
        );
    }
}
#[test]
fn real_recursion_mutual_recursion_nested_calls_and_returns() {
    qualifies(
        "long long factorial(int n){if(n<2)return 1;return n*factorial(n-1);}int main(){return (int)factorial(6);}",
        720,
    );
    qualifies(
        "int fib(int n){if(n<2)return n;return fib(n-1)+fib(n-2);}int main(){return fib(10);}",
        55,
    );
    qualifies(
        "int odd(int n);int even(int n);int even(int n){if(n==0)return 1;return odd(n-1);}int odd(int n){if(n==0)return 0;return even(n-1);}int main(){return even(12)*2+odd(13);}",
        3,
    );
    qualifies(
        "int plus(int a,int b){return a+b;}int inc(int x){return x+1;}int main(){return plus(inc(3),plus(inc(4),inc(5)));}",
        15,
    );
    qualifies(
        "int two(void){return 2;}int three(void){return 3;}int main(){int x=two();int y=three();return x+y;}",
        5,
    );
    qualifies(
        "void nothing(int x){if(x)return;return;}int unused(void){int x=3;}int main(){nothing(1);unused();return 7;}",
        7,
    );
    qualifies(
        "_Bool truth(unsigned long long x){return x;}signed char narrow(int x){return (signed char)x;}int main(){return truth(1ULL<<32)+narrow(255);}",
        0,
    );
}
#[test]
fn every_argument_count_zero_through_twelve_uses_real_abi_homes() {
    let kinds = [
        "signed char",
        "unsigned char",
        "short",
        "unsigned short",
        "int",
        "unsigned int",
        "long",
        "unsigned long",
        "long long",
        "unsigned long long",
        "_Bool",
        "long long",
    ];
    let values = [
        "-2",
        "250",
        "-300",
        "60000",
        "-70000",
        "4294967295u",
        "-90000L",
        "4294967295UL",
        "-5000000000LL",
        "18446744073709551615ULL",
        "23",
        "77LL",
    ];
    let represented = [
        (-2i64) as u64,
        250,
        (-300i64) as u64,
        60000,
        (-70000i64) as u64,
        u32::MAX as u64,
        (-90000i64) as u64,
        u32::MAX as u64,
        (-5000000000i64) as u64,
        u64::MAX,
        1,
        77,
    ];
    for count in 0..=12 {
        let params = if count == 0 {
            "void".into()
        } else {
            (0..count)
                .map(|i| format!("{} p{i}", kinds[i]))
                .collect::<Vec<_>>()
                .join(",")
        };
        let args = values[..count].join(",");
        let calculation = if count == 0 {
            "123ULL".into()
        } else {
            (0..count)
                .map(|i| format!("(unsigned long long)p{i}*{}ULL", i + 1))
                .collect::<Vec<_>>()
                .join("+")
        };
        let expected = if count == 0 {
            123
        } else {
            represented[..count]
                .iter()
                .enumerate()
                .fold(0u64, |sum, (i, value)| {
                    sum.wrapping_add(value.wrapping_mul(i as u64 + 1))
                })
        };
        qualifies(
            &format!(
                "unsigned long long sum({params}){{return {calculation};}}int main(void){{return sum({args})=={expected}ULL?0:1;}}"
            ),
            0,
        );
    }
}
#[test]
fn parameter_scope_constraints_and_wrong_calls_are_rejected() {
    qualifies(
        "int f(int x){{int x=2;x++;}return x;}int main(){return f(9);}",
        9,
    );
    for source in [
        "int f(int x){int x;return 0;}int main(){return 0;}",
        "int f(int){return 0;}int main(){return 0;}",
        "int f(int x){return x;}int main(){return f();}",
        "int f(int x){return x;}int main(){return f(1,2);}",
        "int f(int);long f(int);int main(){return 0;}",
        "int f(void){return;}int main(){return 0;}",
        "void f(void){return 2;}int main(){return 0;}",
        "int f(void){return 1;}int f(void){return 2;}int main(){return 0;}",
        "int main(){int f=0;return f();}",
    ] {
        assert!(
            typed_front::parse(source)
                .and_then(|unit| typed_lower::lower(&unit))
                .is_err(),
            "{source}"
        );
    }
}
#[test]
fn checked_faults_propagate_without_reinterpreting_normal_return_values() {
    for (source, status, diagnostic) in [
        (
            "int fail(int n){if(n==0)return 1/0;return fail(n-1)+1;}int main(){return fail(4);}",
            1,
            "division by zero",
        ),
        (
            "void fail(void){int x;x++;}int main(){fail();return 9;}",
            3,
            "uninitialized",
        ),
        (
            "int fail(void){int x=2147483647;return x+1;}int main(){int x=fail();return x+1;}",
            2,
            "overflow",
        ),
    ] {
        for optimize in [false, true] {
            let built = compile(source, optimize);
            let (actual, stdout, stderr) = execute(&built.image.bytes);
            assert_eq!(actual, status, "{source}");
            assert!(stdout.is_empty());
            assert!(stderr.contains(diagnostic));
        }
    }
    let built = compile("int f(int x){return x;}int main(){return f(8);}", false);
    let bytes = &built.image.bytes;
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let exception_size = u32::from_le_bytes(
        bytes[pe + 24 + 112 + 3 * 8 + 4..pe + 24 + 112 + 3 * 8 + 8]
            .try_into()
            .unwrap(),
    );
    assert_eq!(
        exception_size / 12,
        6,
        "startup, adapter, formatter, writer and two real C functions"
    );
}

#[test]
fn globals_static_locals_tentative_definitions_and_actual_data_widths() {
    for source in [
        "int g;int main(){return g;}",
        "int g=5;int bump(void){return ++g;}int main(){int a=bump();int b=bump();return a+b+g-20;}",
        "int calls;int next(void){static int x=3;calls++;return ++x;}int main(){int a=next();int b=next();return a==4&&b==5&&calls==2?0:1;}",
        "static int g=9;extern int g;static int f(void);int f(void){return g;}int main(){return f()-9;}",
        "extern unsigned long long g;unsigned long long g;unsigned long long g=18446744073709551615ULL;int main(){g++;return g==0?0:1;}",
        "signed char a=127;signed char guard=90;short b=32767;short next=23;int main(){a++;b++;return a==-128&&guard==90&&b==-32768&&next==23?0:1;}",
        "const int g=4;volatile unsigned long long v;int main(){v=(1ULL<<40)+g;return v==1099511627780ULL?0:1;}",
        "int g=2;int f(void){int g=3;{extern int g;g++;}return g;}int main(){return f()==3&&g==3?0:1;}",
        "int g=0;int f(void){g++;}int main(){f();f();return g==2?0:1;}",
        "int f(void){static int x;return ++x;}int h(void){static int x=10;return ++x;}int main(){return f()==1&&h()==11&&f()==2&&h()==12?0:1;}",
    ] {
        qualifies(source, 0);
    }
    for source in [
        "int g=1;int g=2;int main(){}",
        "extern int g;static int g;int main(){}",
        "static int g;int g;int main(){}",
        "int g;long g;int main(){}",
        "int f(void){return 1;}int g=f();int main(){}",
        "int main(){static int g=1;static int g=2;}",
        "const int g=1;int main(){g=2;}",
        "int main(){extern int g=2;}",
        "int main(){static int f(void);}",
    ] {
        assert!(
            typed_front::parse(source)
                .and_then(|unit| typed_lower::lower(&unit))
                .is_err(),
            "{source}"
        );
    }
    let mut c = CompilationContext::for_semantics(Semantics::C11);
    let id = c.add_source(
        "unresolved.c",
        "extern int missing;int main(){return missing;}",
    );
    assert!(
        c.compile_typed(id, &Default::default())
            .unwrap_err()
            .message
            .contains("unresolved global")
    );
}

#[test]
fn pointer_values_literal_storage_global_relocations_and_void_effects() {
    for source in [
        "int g=7;int *p=&g;int main(){return p==&g&&p!=0?0:1;}",
        "void *p=&p;int main(){return p==&p?0:1;}",
        "const char *p=\"hello 世界\";const char *get(void){return p;}int main(){return get()==p&&p?0:1;}",
        "const char *choose(int n){return n?\"abc\":0;}int main(){const char *p=choose(1);return p&&!choose(0)?0:1;}",
        "int empty(const char *p){return !p;}int main(){return empty(0)==1&&empty(\"x\")==0?0:1;}",
        "int g;void set(int x){g=x;return;}int main(){1?set(3):set(9);(void)set(4);return (set(7),g)==7?0:1;}",
        "int main(){char *p;p=0;return p==0&&sizeof((0,\"abc\"))==8?0:1;}",
    ] {
        qualifies(source, 0);
    }
    let built = compile("int g=7;int *p=&g;int main(){return p==&g?0:1;}", false);
    assert!(built.image.sections.iter().any(|s| s.name == ".gdata"));
    let bytes = &built.image.bytes;
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    let relocation_size = u32::from_le_bytes(
        bytes[pe + 24 + 112 + 5 * 8 + 4..pe + 24 + 112 + 5 * 8 + 8]
            .try_into()
            .unwrap(),
    );
    assert!(
        relocation_size >= 12,
        "real static pointer requires a DIR64 relocation block"
    );
    let reloc = built
        .image
        .sections
        .iter()
        .find(|s| s.name == ".reloc")
        .unwrap();
    let raw = reloc.raw_offset as usize;
    let block_size = u32::from_le_bytes(bytes[raw + 4..raw + 8].try_into().unwrap()) as usize;
    assert!(block_size >= 10 && block_size <= relocation_size as usize);
    assert!(
        bytes[raw + 8..raw + block_size]
            .chunks_exact(2)
            .any(|entry| u16::from_le_bytes(entry.try_into().unwrap()) >> 12 == 10),
        "actual DIR64 entry required"
    );
}

#[test]
fn variadic_cursors_read_real_register_homes_and_stack_slots() {
    for count in 0..=12 {
        let values = (0..count)
            .map(|i| format!("{}LL", (i + 1) * 101))
            .collect::<Vec<_>>();
        let args = if values.is_empty() {
            format!("{count}")
        } else {
            format!("{count},{}", values.join(","))
        };
        let expected = (1..=count).sum::<usize>() * 101;
        qualifies(
            &format!(
                "typedef __builtin_va_list va_list;long long sum(int n,...){{va_list ap;__builtin_va_start(ap,n);long long total=0;for(int i=0;i<n;i++)total+=__builtin_va_arg(ap,long long);__builtin_va_end(ap);return total;}}int main(){{return sum({args})=={expected}LL?0:1;}}"
            ),
            0,
        );
    }
    for fixed in 1..=6 {
        let prefix = (0..fixed - 1)
            .map(|i| format!("int x{i}"))
            .collect::<Vec<_>>();
        let mut params = prefix;
        params.push("int n".into());
        let mut args = (0..fixed - 1)
            .map(|i| format!("{}", i + 100))
            .collect::<Vec<_>>();
        args.extend(["3".into(), "11LL".into(), "22LL".into(), "33LL".into()]);
        qualifies(
            &format!(
                "typedef __builtin_va_list va_list;long long sum({},...){{va_list a;__builtin_va_start(a,n);long long total=0;for(int i=0;i<n;i++)total+=__builtin_va_arg(a,long long);__builtin_va_end(a);return total;}}int main(){{return sum({})==66?0:1;}}",
                params.join(","),
                args.join(",")
            ),
            0,
        );
    }
    qualifies(
        "typedef __builtin_va_list va_list;int first(va_list ap){return __builtin_va_arg(ap,int);}int sum(int n,...){va_list a,b;__builtin_va_start(a,n);__builtin_va_copy(b,a);int x=__builtin_va_arg(a,int);int y=__builtin_va_arg(b,int);__builtin_va_end(a);int z=__builtin_va_arg(b,int);__builtin_va_end(b);return x==y&&x==11&&z==22?0:1;}int forward(int n,...){va_list ap;__builtin_va_start(ap,n);int result=first(ap);__builtin_va_end(ap);return result;}int main(){return sum(2,11,22)==0&&forward(1,77)==77?0:1;}",
        0,
    );
    qualifies(
        "typedef __builtin_va_list va_list;int mixed(int n,...){va_list ap;__builtin_va_start(ap,n);int a=__builtin_va_arg(ap,int);unsigned int b=__builtin_va_arg(ap,unsigned int);long c=__builtin_va_arg(ap,long);unsigned long long d=__builtin_va_arg(ap,unsigned long long);const char *p=__builtin_va_arg(ap,const char*);void *z=__builtin_va_arg(ap,void*);__builtin_va_end(ap);return a==-3&&b==4294967295u&&c==-9L&&d==18446744073709551615ULL&&p&&z==0?0:1;}int main(){signed char a=-3;return mixed(6,a,4294967295u,-9L,18446744073709551615ULL,\"text\",(void*)0);}",
        0,
    );
}

#[test]
fn invalid_variadic_contexts_types_and_ended_cursors_are_checked() {
    for source in [
        "typedef __builtin_va_list va_list;int f(int n){va_list ap;__builtin_va_start(ap,n);return 0;}int main(){}",
        "typedef __builtin_va_list va_list;int f(int n,int m,...){va_list ap;__builtin_va_start(ap,n);return 0;}int main(){}",
        "typedef __builtin_va_list va_list;int f(int n,...){va_list ap;{int n=1;__builtin_va_start(ap,n);}return 0;}int main(){}",
        "typedef __builtin_va_list va_list;int f(register int n,...){va_list ap;__builtin_va_start(ap,n);return 0;}int main(){}",
        "typedef __builtin_va_list va_list;int f(short n,...){va_list ap;__builtin_va_start(ap,n);return 0;}int main(){}",
        "typedef __builtin_va_list va_list;int f(int n,...){va_list ap;__builtin_va_start(ap,n);return __builtin_va_arg(ap,char);}int main(){}",
        "int main(){int ap;__builtin_va_end(ap);}",
    ] {
        assert!(typed_front::parse(source).is_err(), "{source}");
    }
    let source = "typedef __builtin_va_list va_list;int f(int n,...){va_list ap;__builtin_va_start(ap,n);__builtin_va_end(ap);return __builtin_va_arg(ap,int);}int main(){return f(1,7);}";
    for optimize in [false, true] {
        let built = compile(source, optimize);
        let (status, stdout, stderr) = execute(&built.image.bytes);
        assert_eq!(status, 3);
        assert!(stdout.is_empty());
        assert!(stderr.contains("uninitialized"));
    }
}

#[test]
fn owned_c_runtime_uses_real_variadic_abi_and_computed_counts() {
    let prelude = "int printf(const char*,...);unsigned long long strlen(const char*);";
    for optimize in [false, true] {
        let source = format!(
            "{prelude} int report(int n){{return printf(\"n=%d %s\\n\",n,\"hello\");}} int main(){{int n=report(-42);return n+(int)strlen(\"abc\");}}"
        );
        assert_eq!(
            execute(&compile(&source, optimize).image.bytes),
            (15, "n=-42 hello\n".into(), String::new())
        );
        let source = format!(
            "{prelude} int g;int bump(){{g=g+1;return g;}}int main(){{int n=printf(\"%d %d %d %d %d %d %d %s\",1,2,3,4,5,6,7,\"end\",bump());return n+g;}}"
        );
        assert_eq!(
            execute(&compile(&source, optimize).image.bytes),
            (18, "1 2 3 4 5 6 7 end".into(), String::new())
        );
        let source = format!(
            "{prelude} int main(){{int n=printf(\"é%s\",\"a\\0ignored\");return n+(int)strlen(\"ab\\0tail\");}}"
        );
        assert_eq!(
            execute(&compile(&source, optimize).image.bytes),
            (5, "éa".into(), String::new())
        );
    }
    qualifies(
        "int printf(int n){return n+1;}int strlen(void){return 9;}int main(){return printf(strlen());}",
        10,
    );
}
#[test]
fn runtime_prototypes_formats_and_lookup_are_checked() {
    for source in [
        "int printf(const char*,...);int main(){return printf(\"%d\");}",
        "int printf(const char*,...);int main(){return printf(\"%s\",1);}",
        "int printf(const char*,...);int main(){return printf(\"%d\",1LL);}",
        "int printf(const char*,...);int main(){return printf(\"%%\");}",
        "int printf(const char*,...);int main(){char*p=\"ok\";return printf(p);}",
        "static int printf(const char*,...);int main(){return printf(\"ok\");}",
        "int printf(char*,...);int main(){return printf(\"ok\");}",
        "int strlen(const char*);int main(){return strlen(\"ok\");}",
        "int printf(const char*,...);int main(){int printf=1;return printf(\"ok\");}",
        "int sum(int a[],...){__builtin_va_list p;__builtin_va_start(p,a);return 0;}int main(){return 0;}",
    ] {
        let mut context = CompilationContext::for_semantics(Semantics::C11);
        let id = context.add_source("invalid.c", source);
        assert!(
            context
                .compile_typed(id, &NativeCompileOptions::default())
                .is_err(),
            "accepted {source}"
        );
    }
}
