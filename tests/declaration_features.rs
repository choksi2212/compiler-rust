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
fn static_assertions_analyze_file_block_and_record_declarations() {
    qualifies(
        "_Static_assert(sizeof(long)==4,\"LLP64\");struct S{_Static_assert(sizeof(int)==4,\"int width\");int x;};int main(){_Static_assert(sizeof(struct S)==4,\"record layout\");return 42;}",
        42,
    );
    for source in [
        "_Static_assert(0,\"required diagnostic\");int main(){return 0;}",
        "int n;_Static_assert(n,\"not constant\");int main(){return 0;}",
        "int main(){_Static_assert(0,\"block failure\");return 0;}",
        "struct S{_Static_assert(0,\"member failure\");int x;};int main(){return 0;}",
        "_Static_assert(1,9);int main(){return 0;}",
    ] {
        assert!(
            typed_front::parse(source)
                .and_then(|u| typed_lower::lower(&u))
                .is_err(),
            "{source}"
        );
    }
    let error = typed_front::parse("_Static_assert(0,\"required diagnostic\");").unwrap_err();
    assert!(error.contains("required diagnostic"));
}

#[test]
fn generic_selection_preserves_types_lvalues_and_unevaluated_expressions() {
    for source in [
        "int main(){int count=0;int n=_Generic(count++,int:42,default:0);return count==0?n:0;}",
        "int main(){int count=0;int n=_Generic(1,int:42,default:count++);return count==0?n:0;}",
        "int not_defined(void);int main(){return _Generic(not_defined(),int:42,default:0);}",
        "int main(){int x=1;int y=2;_Generic(0,int:x,default:y)=42;return x;}",
        "int main(){const int x=7;return _Generic(x,int:42,const int:1,default:0);}",
        "int main(){unsigned char x=1;return _Generic(x,unsigned char:42,int:0,default:1);}",
        "int main(){int a[3];return _Generic(a,int*:42,int[3]:1,default:0);}",
        "int f(int n){return n;}int main(){return _Generic(f,int(*)(int):42,default:0);}",
        "struct S{unsigned int n:3;};int main(){struct S s;return _Generic(s.n,unsigned int:42,int:1,default:0);}",
        "struct S{int x;};int main(){struct S s;return _Generic(s,struct S:42,default:0);}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "int main(){return _Generic(0,int:1,int:2);}",
        "typedef int T;int main(){return _Generic(0,int:1,T:2);}",
        "int main(){return _Generic(0,default:1,default:2);}",
        "int main(){return _Generic(0,char:1);}",
        "int main(){return _Generic(0,void:1,default:2);}",
        "int main(){return _Generic(0,int:42,default:undeclared);}",
    ] {
        assert!(
            typed_front::parse(source)
                .and_then(|u| typed_lower::lower(&u))
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn explicit_object_member_and_type_alignments_reach_real_addresses() {
    for source in [
        "char lead;_Alignas(16) int global;int main(){char local_lead=1;_Alignas(16) int x=42;return ((unsigned long long)&global%16)==0&&((unsigned long long)&x%16)==0?x+local_lead-1:0;}",
        "struct S{char a;_Alignas(16) int x;char tail;};int main(){struct S s;s.x=42;return sizeof(struct S)==32&&_Alignof(struct S)==16&&((unsigned long long)&s.x%16)==0?s.x:0;}",
        "int main(){char lead=1;int _Alignas(16) x=41;return ((unsigned long long)&x%16)==0?lead+x:0;}",
        "_Alignas(16) extern int x;int x;int main(){return ((unsigned long long)&x%16)==0?42:0;}",
        "int main(){_Alignas(8) _Alignas(16) int x=42;return ((unsigned long long)&x%16)==0?x:0;}",
        "int main(){_Alignas(0) int x=42;return x;}",
        "int main(){_Alignas(long long) char bytes[8];return ((unsigned long long)bytes%8)==0?42:0;}",
    ] {
        qualifies(source, 42);
    }
    let unit = typed_front::parse("int main(){_Alignas(long long) char a[8];return 0;}").unwrap();
    assert_eq!(
        unit.declarations
            .iter()
            .find(|d| d.token.text == "a")
            .unwrap()
            .alignment,
        Some(8)
    );
    for source in [
        "int main(){_Alignas(1) int x;return 0;}",
        "int main(){_Alignas(4) int*p;return 0;}",
        "int main(){_Alignas(3) int x;return 0;}",
        "int main(){_Alignas(32) int x;return 0;}",
        "_Alignas(16) typedef int T;int main(){return 0;}",
        "int main(){_Alignas(16) register int x;return 0;}",
        "int main(){_Alignas(0) register int x;return 0;}",
        "_Alignas(0) int f(void){return 0;}int main(){return f();}",
        "int f(_Alignas(16) int x){return x;}int main(){return f(0);}",
        "struct S{_Alignas(16) int x:3;};int main(){return 0;}",
        "struct S{_Alignas(0) int x:3;};int main(){return 0;}",
        "_Alignas(16) int x;_Alignas(8) int x;int main(){return x;}",
    ] {
        assert!(
            typed_front::parse(source)
                .and_then(|u| typed_lower::lower(&u))
                .is_err(),
            "{source}"
        );
    }
}

#[test]
fn alignment_ir_metadata_and_invalid_claims_are_verified() {
    let built = compile(
        "char leading;_Alignas(16) int g;int main(){char first;_Alignas(16) int x=42;return x;}",
        false,
    );
    let global = built.ir.globals.iter().position(|g| g.name == "g").unwrap();
    assert_eq!(built.ir.globals[global].alignment, Some(16));
    let function = built
        .ir
        .functions
        .iter()
        .position(|f| f.name == "main")
        .unwrap();
    let local = built.ir.functions[function]
        .locals
        .iter()
        .position(|l| l.alignment == Some(16))
        .unwrap();
    let mut bad = built.ir.clone();
    bad.globals[global].alignment = Some(3);
    assert!(compiler::typed_ir::verify(&bad).is_err());
    let mut bad = built.ir.clone();
    bad.functions[function].locals[local].alignment = Some(32);
    assert!(compiler::typed_ir::verify(&bad).is_err());
    let mut bad = built.ir;
    bad.functions[function].locals[local].alignment = Some(1);
    assert!(compiler::typed_ir::verify(&bad).is_err());
}

#[test]
fn hostile_generic_association_counts_fail_at_the_documented_budget() {
    let mut source = String::from("int main(){return _Generic(0,");
    // Distinct complete array association types prevent the duplicate-type
    // diagnostic from hiding the aggregate association-count budget.
    for n in 1..=1025 {
        source.push_str(&format!("int[{n}]:0,"));
    }
    source.push_str("default:0);}");
    let error = typed_front::parse(&source).unwrap_err();
    assert!(error.contains("generic association budget"), "{error}");
}

#[test]
fn recursive_alignment_type_specifiers_reject_before_stack_exhaustion() {
    let source = format!(
        "{}int{} int value;int main(){{return 0;}}",
        "_Alignas(".repeat(1000),
        ")".repeat(1000)
    );
    let error = typed_front::parse(&source).unwrap_err();
    assert!(error.contains("alignment nesting limit"), "{error}");
}
