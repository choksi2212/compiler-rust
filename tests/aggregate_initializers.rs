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
fn aggregate_brace_initialization_remainders_bitfields_and_copies() {
    for source in [
        "struct S{int x;short y;char z;};int main(){struct S s={10,12,20};return s.x+s.y+s.z;}",
        "struct S{int x;int y;int z;};struct S g={42};int main(){return g.x+g.y+g.z;}",
        "struct I{int x;int y;};struct S{struct I i;int z;};int main(){struct S s={10,12,20};return s.i.x+s.i.y+s.z;}",
        "struct S{int a[2];int z;};int main(){struct S s={{10,12},20};return s.a[0]+s.a[1]+s.z;}",
        "struct B{const int s:3;unsigned int u:4;int z;};int main(){struct B b={-2,12,32};return b.s+b.u+b.z;}",
        "struct B{int s:3;unsigned int u:4;int z;};struct B b={-2,12,32};int main(){return b.s+b.u+b.z;}",
        "union U{int x;long long y;};union U u={42};int main(){return u.x;}",
        "struct S{int x;int y;};int main(){struct S a={10,32};struct S b=a;a.x=100;return b.x+b.y;}",
    ] {
        qualifies(source, 42);
    }
}
#[test]
fn nested_designators_continuation_inferred_bounds_and_union_replacement() {
    for source in [
        "int a[]={[4]=10,[1]=12,20};int main(){return sizeof(a)==20?a[4]+a[1]+a[2]+a[0]+a[3]:0;}",
        "struct I{int x;int y;};struct S{struct I i;int z;};struct S s={.i.x=10,12,20};int main(){return s.i.x+s.i.y+s.z;}",
        "struct S{int a[2];int z;};int main(){struct S s={.a[1]=22,.z=20};return s.a[0]+s.a[1]+s.z;}",
        "struct S{int x;int y;};struct S a[2]={[1].y=20,[0]={10,12}};int main(){return a[0].x+a[0].y+a[1].x+a[1].y;}",
        "struct S{int x;int y;};int main(){struct S a={1,20};struct S b={.x=1,.y=20,.x=22};return b.x+b.y;}",
        "union U{void*p;unsigned int n;};union U u={.p=&u,.n=42};int main(){return u.n;}",
        "struct S{union{int x;int y;};int z;};int main(){struct S s={.y=12,.z=30};return s.y+s.z;}",
        "char a[2][4]={[1]=\"abc\"};int main(){return a[0][0]==0&&a[1][3]==0?a[1][1]-56:0;}",
    ] {
        qualifies(source, 42);
    }
}
#[test]
fn compound_literals_have_real_static_and_block_object_identity() {
    for source in [
        "struct S{int x;int y;};int main(){struct S s=(struct S){10,32};return s.x+s.y;}",
        "struct S{int x;int y;};struct S*p=&(struct S){.y=32,.x=10};int main(){return p->x+p->y;}",
        "int*p=(int[]){10,12,20};int main(){return p[0]+p[1]+p[2];}",
        "int main(){int*p=&(int){40};*p+=2;return *p;}",
        "struct S{int x;int y;};int main(){return (struct S){10,32}.x+(struct S){10,32}.y;}",
        "struct I{int x;int y;};struct S{struct I i;int z;};int main(){struct S s={.i=(struct I){10,20},.i.x=12,.z=10};return s.i.x+s.i.y+s.z;}",
        "int main(){int*p=0;int*q=0;for(int i=0;i<2;i++){q=&(int){i};if(i==0)p=q;}return p==q&&*q==1?42:0;}",
    ] {
        qualifies(source, 42);
    }
}
#[test]
fn invalid_initializers_and_static_nonconstants_are_rejected() {
    for source in [
        "struct S{int x;};struct S g=(struct S){42};int main(){return 0;}",
        "struct S{int x;};int main(){struct S s={1,2};return 0;}",
        "struct S{int x;};int main(){struct S s={.missing=42};return 0;}",
        "int main(){int a[2]={[2]=1};return 0;}",
        "int main(){int a[2]={[-1]=1};return 0;}",
        "int main(){int n={.x=1};return 0;}",
        "struct S{int n;int a[];};int main(){struct S s={.a={1}};return 0;}",
        "struct S{int x;};int f(void){return 1;}struct S s={f()};int main(){return 0;}",
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
fn initializer_mutations_and_malformed_ordered_writes_are_diagnosed() {
    use compiler::{native_runtime::RuntimeOptions, typed_image, typed_ir, typed_opt};
    let seeds = [
        "struct S{int x;int a[2];};struct S g={.a[1]=3,.x=4};int main(){struct S s={.x=1,.a={2,3}};return s.x+g.x;}",
        "struct B{int x:3;unsigned int y:5;};int main(){struct B*p=&(struct B){.x=-1,.y=4};return p->x+p->y;}",
        "int*p=(int[]){[2]=3,[0]=1};int main(){int*q=&(int){2};return p[2]+*q;}",
    ];
    let pipeline = |source: &str| -> Result<(), String> {
        let unit = typed_front::parse(source)?;
        if unit.main().is_none() {
            return Ok(());
        }
        let ir = typed_lower::lower(&unit)?;
        typed_ir::verify(&ir)?;
        let optimized = typed_opt::optimize(&ir)?;
        typed_image::build(&optimized, &RuntimeOptions::default()).map(|_| ())
    };
    for seed in seeds {
        for at in 0..seed.len() {
            for replacement in ["", ":", ";", "*", "++", ".", "]", "}"] {
                let mut source = seed.to_string();
                source.replace_range(at..at + 1, replacement);
                assert!(
                    std::panic::catch_unwind(|| pipeline(&source)).is_ok(),
                    "{source}"
                );
            }
        }
    }
    let unit =
        typed_front::parse("struct S{int x;int y;};struct S s={.x=42};int main(){return s.x;}")
            .unwrap();
    let module = typed_lower::lower(&unit).unwrap();
    let mut bad = module.clone();
    let typed_ir::GlobalInitializer::Object(writes) = &mut bad.globals[0].initial else {
        panic!()
    };
    writes[0].path = vec![typed_ir::InitializerStep::Member(999)];
    assert!(typed_ir::verify(&bad).is_err());
    let mut bad = module;
    let typed_ir::GlobalInitializer::Object(writes) = &mut bad.globals[0].initial else {
        panic!()
    };
    writes[0].initial = typed_ir::GlobalInitializer::Address {
        symbol: typed_ir::SymbolId(999),
        addend: 0,
    };
    assert!(typed_ir::verify(&bad).is_err());
}
