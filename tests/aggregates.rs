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
fn enum_constants_promotions_scopes_calls_and_object_storage() {
    for source in [
        "enum E{A=-2,B,C=40,D};enum E global=D;enum E f(enum E x){return x+1;}int main(){enum E value=f(global);return value;}",
        "enum E{A=40,B};int main(){int n=B;{enum E{A=1,B=2};n+=A;}return n;}",
        "typedef enum E{A=42} E;int main(){E a[2]={A,0};E*p=a;return sizeof(E)==4&&_Alignof(E)==4&&a[1]==0?*p:0;}",
        "enum E{FIRST=1,LAST=2147483647};int main(){enum E x=FIRST;switch(x){case FIRST:return 42;default:return 0;}}",
        "int f(enum E{A=40,B} x){enum E y=x;return y+A;}int main(){return f(2);}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "enum E;",
        "enum E{A=2147483648ULL};",
        "enum E{A=2147483647,B};",
        "enum E{A,A};",
        "enum E{};",
        "enum E{A};struct E*p;",
        "enum E{A};enum E{B};",
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
fn record_layout_queries_execute_with_actual_target_sizes() {
    qualifies(
        "struct S{char a;int b;short c;};union U{long long wide;char bytes[3];};struct Node{int n;struct Node*next;};int main(){struct S s;return sizeof(s)==12&&_Alignof(struct S)==4&&sizeof(union U)==8&&sizeof(struct Node)==16?42:0;}",
        42,
    );
}

#[test]
fn real_struct_union_members_anonymous_members_and_pointer_arrows() {
    for source in [
        "struct S{char a;int b;short c;};int main(){struct S s;s.a=10;s.b=20;s.c=12;return s.a+s.b+s.c;}",
        "struct S{char a;int b;short c;};struct S g;int*p=&g.b;int main(){*p=42;return g.a==0&&g.c==0?g.b:0;}",
        "struct S{int a[3];};int main(){struct S s;s.a[0]=10;s.a[1]=12;s.a[2]=20;return s.a[0]+s.a[1]+s.a[2];}",
        "struct S{int n;struct S*next;};void set(struct S*p){p->n=42;p->next=p;}int main(){struct S s;set(&s);return s.next->n;}",
        "union U{unsigned int n;unsigned char byte;};int main(){union U u;u.n=0x1234562au;return u.byte;}",
        "struct S{char prefix;union{int x;struct{short y;short z;};};};int main(){struct S s;s.x=0;s.y=10;s.z=32;return s.y+s.z;}",
        "struct S{int x;};int main(){struct S a[2];a[0].x=10;a[1].x=32;return a[0].x+a[1].x;}",
        "struct S{int n;int a[];};void*malloc(unsigned long long);void free(void*);int main(){struct S*p=malloc(sizeof(struct S)+2*sizeof(int));if(!p)return 1;p->n=2;p->a[0]=10;p->a[1]=32;int n=p->a[0]+p->a[1];free(p);return n;}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "struct S{int x;};int main(){struct S s;return s.missing;}",
        "int main(){int n;return n.x;}",
        "struct S{int x;};int main(){const struct S*s=0;s->x=2;return 0;}",
        "struct S{const int x;};int main(){struct S s;s.x=1;return 0;}",
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
fn native_bitfield_masking_sign_extension_promotions_and_single_evaluation() {
    for source in [
        "struct B{unsigned int a:3,b:5;int s:4;};int main(){struct B x;x.a=7;x.b=21;x.s=-2;x.a=1;return x.a==1&&x.b==21&&x.s==-2?42:0;}",
        "struct B{unsigned char a:4,b:4;unsigned char guard;};int main(){struct B x;x.a=15;x.b=10;x.guard=85;x.a=1;unsigned char*p=(unsigned char*)&x;return p[0]==161&&x.guard==85?42:0;}",
        "struct B{signed int n:3;};int main(){struct B x;int n=(x.n=7);return n==-1&&x.n==-1?42:0;}",
        "struct B{unsigned int n:3;};int main(){struct B x;x.n=7;int old=x.n++;int next=++x.n;x.n+=10;return old==7&&next==1&&x.n==3?42:0;}",
        "struct B{unsigned int n:3;unsigned int full:32;};int main(){struct B x;x.n=0;x.full=0;return (x.n-1)<0&&(x.full-1)>0&&sizeof(+x.n)==4?42:0;}",
        "struct B{_Bool b:1;};int main(){struct B x;x.b=9;int a=x.b++;int b=--x.b;return a==1&&b==0?42:0;}",
        "struct B{unsigned int n:3;};struct B g;int calls;struct B*get(void){calls++;return &g;}int main(){get()->n+=2;int n=get()->n++;return calls==2&&n==2&&g.n==3?42:0;}",
        "struct B{unsigned long long n:63;};int main(){struct B x;x.n=9223372036854775807ULL;x.n=5;return x.n==5?42:0;}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "struct B{int n:3;};int main(){struct B x;return sizeof(x.n);}",
        "struct B{int n:3;};int main(){struct B x;int*p=&x.n;return 0;}",
        "struct B{const int n:3;};int main(){struct B x;x.n=1;return 0;}",
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
fn aggregate_value_copies_use_real_byte_homes_and_preserve_source_objects() {
    for source in [
        "struct S{int x;short y;char z;};int main(){struct S a;struct S b;a.x=10;a.y=12;a.z=20;b=a;a.x=99;return b.x+b.y+b.z;}",
        "struct S{char a[3];};int main(){struct S a;struct S b;a.a[0]=10;a.a[1]=12;a.a[2]=20;b=a;return b.a[0]+b.a[1]+b.a[2];}",
        "struct S{long long x;long long y;int z;};struct S g;int main(){struct S a;a.x=10;a.y=12;a.z=20;g=a;struct S b=g;return (int)(b.x+b.y+b.z);}",
        "struct S{int x;};struct Outer{struct S s;};int main(){struct Outer a;struct Outer b;a.s.x=42;b.s=a.s;a.s.x=0;return b.s.x;}",
        "struct S{int x;};int main(){struct S a;struct S b;struct S c;a.x=42;b.x=0;c=1?a:b;c=c;return c.x;}",
        "struct S{int x;};int main(){struct S a;struct S b;a.x=42;return (b=a).x;}",
        "union U{int x;char c;};int main(){union U a;union U b;a.x=42;b=a;return b.x;}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "struct S{const int x;};int main(){struct S a;struct S b;a=b;return 0;}",
        "struct S{int x;};int main(){struct S a;struct S b;(b=a).x=4;return 0;}",
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
fn aggregate_arguments_returns_hidden_storage_and_variadic_values() {
    for source in [
        "struct S{int x;int y;};struct S f(struct S s){s.x+=2;return s;}int main(){struct S a;a.x=10;a.y=30;struct S b=f(a);return b.x+b.y==42&&a.x==10?42:0;}",
        "struct S{int x;int y;int z;};struct S f(struct S s,int n){s.x+=n;return s;}int main(){struct S a;a.x=10;a.y=12;a.z=18;struct S b=f(a,2);return b.x+b.y+b.z==42&&a.x==10?42:0;}",
        "struct S{char c[3];};struct S f(struct S s){s.c[1]+=2;return s;}int main(){struct S a;a.c[0]=10;a.c[1]=10;a.c[2]=20;return f(a).c[0]+f(a).c[1]+f(a).c[2];}",
        "struct S{long long a;long long b;};struct S f(int n,struct S s,int x,int y,int z,int w){s.a+=n+x+y+z+w;return s;}int main(){struct S s;s.a=1;s.b=20;struct S r=f(1,s,2,3,5,10);return (int)(r.a+r.b);}",
        "struct S{int a;int b;int c;};struct S f(struct S s){s.a++;return s;}int main(){struct S s;s.a=10;s.b=10;s.c=20;struct S(*p)(struct S)=f;return p(p(s)).a+s.b+s.c;}",
        "struct S{int a;};int f(int n,...){__builtin_va_list ap;__builtin_va_start(ap,n);struct S s=__builtin_va_arg(ap,struct S);__builtin_va_end(ap);return s.a;}int main(){struct S s;s.a=42;return f(1,s);}",
        "struct S{int a;int b;int c;};struct S f(int n,...){__builtin_va_list ap;__builtin_va_start(ap,n);struct S s=__builtin_va_arg(ap,struct S);__builtin_va_end(ap);s.a+=n;return s;}int main(){struct S s;s.a=10;s.b=10;s.c=20;return f(2,s).a+s.b+s.c;}",
    ] {
        qualifies(source, 42);
    }
}

#[test]
fn aggregate_images_and_value_snapshots_are_deterministic() {
    let source = "struct S{long long x;int y;};struct S f(struct S s){s.y+=2;return s;}int main(){struct S a;a.x=10;a.y=30;struct S b=f(a);return (int)b.x+b.y;}";
    for optimize in [false, true] {
        let first = compile(source, optimize);
        let second = compile(source, optimize);
        assert_eq!(first.image.bytes, second.image.bytes);
        assert_eq!(
            execute(&first.image.bytes),
            (42, String::new(), String::new())
        );
    }
}

#[test]
fn faculty_demos_compile_and_execute_real_record_and_heap_algorithms() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for name in ["student_records", "linked_list"] {
        let source = fs::read_to_string(root.join(format!("examples/{name}.c"))).unwrap();
        let expected = fs::read_to_string(root.join(format!("examples/{name}.expected")))
            .unwrap()
            .replace("\r\n", "\n");
        for optimize in [false, true] {
            let built = compile(&source, optimize);
            assert_eq!(
                execute(&built.image.bytes),
                (0, expected.clone(), String::new()),
                "{name} opt={optimize}"
            );
        }
    }
}
