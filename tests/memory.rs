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
fn actual_scalar_addresses_dereference_and_alias_stores() {
    for source in [
        "int main(){int x;int *p=&x;*p=42;return x;}",
        "int global;void set(int*p,int n){*p=n;}int main(){int x=1;set(&x,20);set(&global,22);return x+global;}",
        "int main(){short x=-2;short *p=&x;*p=42;return x;}",
        "int main(){signed char x=1;char *p=(char*)&x;*p=42;return x;}",
        "int main(){unsigned long long x=0;unsigned long long *p=&x;*p=1099511627818ULL;return (int)x;}",
        "int main(){int x=42;int*p=&x;int**q=&p;return **q;}",
        "int main(){int x=42;const int*p=&x;return *p;}",
        "int main(){volatile int x;volatile int*p=&x;*p=42;return x;}",
        "int main(){int x=42;int*p=&x;return &*p==p?x:0;}",
    ] {
        qualifies(source, 42);
    }
}
#[test]
fn invalid_object_address_and_dereference_are_diagnosed() {
    for source in [
        "int main(){return *1;}",
        "int main(){void*p=0;return *p;}",
        "int main(){const int x=1;const int*p=&x;*p=2;return x;}",
        "int main(){register int x=1;int*p=&x;return 0;}",
        "int main(){int*p=&42;return 0;}",
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
fn indirect_modifications_evaluate_addresses_once_and_normalize() {
    qualifies(
        "int g=10;int count;int* get(void){count++;return &g;}int main(){*get()+=3;int a=(*get())++;int b=++*get();return count==3&&g==15&&a==13&&b==15?0:1;}",
        0,
    );
    qualifies(
        "int main(){unsigned char x=255;unsigned char*p=&x;int a=(*p)++;int b=++*p;*p+=260;return a==255&&b==1&&x==5?0:1;}",
        0,
    );
    qualifies(
        "int main(){unsigned long long x=0x123456789ULL;unsigned long long*p=&x;*p<<=2;*p^=7;*p/=3;*p%=97;return x==((0x123456789ULL<<2)^7)/3%97?0:1;}",
        0,
    );
}

#[test]
fn scaled_pointer_offsets_differences_and_one_past_comparisons() {
    qualifies(
        "int main(){int x=42;int*p=&x;int*q=p+1;return q-p==1&&p-q==-1&&p<q&&q>p&&q>=q&&p<=q?*p:0;}",
        42,
    );
    qualifies(
        "int main(){short x=42;short*p=&x;short*q=p++;p-=1;q+=1;return p==&x&&q-p==1&&sizeof(*p)==2?x:0;}",
        42,
    );
    qualifies(
        "int main(){long long x=42;long long*p=&x;long long*q=1+p;q--;return q==p?*q:0;}",
        42,
    );
    qualifies(
        "int main(){unsigned int x=0x12345678u;unsigned char*p=(unsigned char*)&x;return *p==0x78&&*(p+1)==0x56&&*(p+2)==0x34&&*(p+3)==0x12?42:0;}",
        42,
    );
    for source in [
        "int main(){void*p=0;p++;}",
        "int main(){int*p=0;return p<0;}",
        "int main(){int*p=0;return p+p;}",
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
fn function_addresses_callbacks_global_relocations_and_indirect_calls() {
    for source in [
        "int f(int x){return x+1;}int main(){int(*p)(int)=f;return p(41);}",
        "int f(int x){return x+1;}int main(){int(*p)(int)=&f;return (*p)(41);}",
        "int f(int x){return x+1;}int(*p)(int)=f;int main(){return p(41);}",
        "int plus(int x,int y){return x+y;}int apply(int(*p)(int,int),int x){return p(x,12);}int main(){return apply(plus,30);}",
        "void set(int*p){*p=42;}int main(){int n=0;void(*p)(int*)=set;p(&n);return n;}",
        "long long many(int a,int b,int c,int d,int e,int f){return a+b+c+d+e+f;}int main(){long long(*p)(int,int,int,int,int,int)=many;return (int)p(2,4,6,8,10,12);}",
    ] {
        qualifies(source, 42);
    }
}

#[test]
fn fixed_and_multidimensional_array_storage_and_row_scaling() {
    for source in [
        "int main(){int a[4];for(int i=0;i<4;i++)a[i]=i*7;return a[1]+a[2]+a[3];}",
        "int a[3];int main(){a[1]=42;return a[0]+a[1]+a[2];}",
        "int main(){int a[2][3];for(int i=0;i<2;i++)for(int j=0;j<3;j++)a[i][j]=i*10+j;return sizeof(a)==24&&sizeof(a[0])==12&&a[1][2]==12&&(&a[1]-&a[0])==1?42:0;}",
        "int sum(int a[],int n){int s=0;for(int i=0;i<n;i++)s+=a[i];return s;}int main(){int a[3];a[0]=10;a[1]=12;a[2]=20;return sum(a,3);}",
        "int main(){char a[3];a[0]=40;a[1]=2;a[2]=0;char*p=a;return *p+1[p];}",
        "int main(){int a[1];a[0]=42;int(*p)[1]=&a;return (*p)[0];}",
        "int main(){return \"abc\"[1]-56;}",
    ] {
        qualifies(source, 42);
    }
}

#[test]
fn array_initializers_inferred_bounds_zero_fill_and_mutable_strings() {
    for source in [
        "int main(){int a[4]={10,12,20};return a[0]+a[1]+a[2]+a[3];}",
        "int a[]={10,12,20};int main(){return sizeof(a)==12?a[0]+a[1]+a[2]:0;}",
        "int main(){int a[][3]={{10,2},{20,10}};return sizeof(a)==24?a[0][0]+a[0][1]+a[0][2]+a[1][0]+a[1][1]+a[1][2]:0;}",
        "int a[2][3]={10,2,0,20,10};int main(){return a[0][0]+a[0][1]+a[1][0]+a[1][1]+a[1][2];}",
        "int main(){char a[]=\"abc\";a[1]='Z';return sizeof(a)==4&&a[1]=='Z'&&a[3]==0?42:0;}",
        "char a[3]=\"abc\";int main(){return sizeof(a)==3&&a[2]=='c'?42:0;}",
        "int main(){const char a[5]={\"ab\"};return a[0]=='a'&&a[1]=='b'&&a[2]==0&&a[4]==0?42:0;}",
        "int g=42;int *a[2]={&g,0};int main(){return a[1]==0?*a[0]:0;}",
        "int f(int x){return x+1;}int(*a[2])(int)={f,0};int main(){return a[0](41);}",
        "int main(){int x=10;int a[3]={x++,x++,20};return x==12?a[0]+a[1]+a[2]+1:0;}",
    ] {
        qualifies(source, 42);
    }
}

#[test]
fn static_array_address_offsets_and_composite_bounds() {
    for source in [
        "int a[3]={10,20,42};int*p=a+2;int main(){return *p;}",
        "int a[3]={10,20,42};int*p=&a[2];int main(){return *p;}",
        "const char*p=\"abc\"+1;int main(){return *p-56;}",
        "int a[3];extern int a[];int main(){a[2]=42;return sizeof(a)==12?a[2]:0;}",
        "int a[3];int a[]={42};int main(){return sizeof(a)==12&&a[1]==0?a[0]:0;}",
        "int a[];int main(){a[0]=42;return a[0];}",
        "extern int unused[];int main(){return 42;}",
    ] {
        qualifies(source, 42);
    }
    for source in [
        "int main(){int a[2]={1,2,3};return 0;}",
        "int main(){char a[2]=\"abc\";return 0;}",
        "int main(){int a[]={};return 0;}",
        "int main(){const int a[2]={1,2};a[0]=3;return 0;}",
        "int f(){return 1;}int a[1]={f()};int main(){return 0;}",
        "int main(){int a[];return 0;}",
        "int main(){int a[2];int b[2];a=b;return 0;}",
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
fn owned_memory_and_string_helpers_operate_on_runtime_bytes() {
    let prelude = "void*memcpy(void*,const void*,unsigned long long);void*memmove(void*,const void*,unsigned long long);void*memset(void*,int,unsigned long long);char*strcpy(char*,const char*);int strcmp(const char*,const char*);unsigned long long strlen(const char*);";
    for body in [
        "char a[8]={0};char b[8]=\"hello\";void*p=memcpy(a,b,6);return p==a&&strcmp(a,b)==0&&strlen(a)==5?42:0;",
        "char a[8]=\"abcdef\";memmove(a+2,a,5);return a[2]=='a'&&a[6]=='e'?42:0;",
        "char a[8]=\"abcdef\";memmove(a,a+2,5);return strcmp(a,\"cdef\")==0?42:0;",
        "unsigned char a[5];void*p=memset(a,0x1234,5);return p==a&&a[0]==0x34&&a[4]==0x34?42:0;",
        "char a[4];char*p=strcpy(a,\"abc\");return p==a&&a[3]==0&&strcmp(a,\"abd\")<0&&strcmp(a,\"abb\")>0?42:0;",
        "return memcpy(0,0,0)==0&&memmove(0,0,0)==0&&memset(0,0,0)==0?42:0;",
        "char a[]={-1,0};char b[]={1,0};return strcmp(a,b)>0?42:0;",
    ] {
        qualifies(&format!("{prelude} int main(){{{body}}}"), 42);
    }
}

#[test]
fn owned_allocator_alignment_zeroing_reallocation_and_failures() {
    let prelude = "void*malloc(unsigned long long);void free(void*);void*calloc(unsigned long long,unsigned long long);void*realloc(void*,unsigned long long);";
    for body in [
        "int*p=malloc(4*12);if(!p)return 1;for(int i=0;i<12;i++)p[i]=i*3;int ok=p[11]==33&&((unsigned long long)p%16)==0;free(p);return ok?42:0;",
        "unsigned char*p=calloc(257,3);if(!p)return 1;int ok=1;for(int i=0;i<771;i++)if(p[i])ok=0;free(p);return ok?42:0;",
        "char*p=malloc(8);if(!p)return 1;for(int i=0;i<8;i++)p[i]=i+1;char*q=realloc(p,100);if(!q){free(p);return 2;}int ok=1;for(int i=0;i<8;i++)if(q[i]!=i+1)ok=0;char*r=realloc(q,4);if(!r){free(q);return 3;}for(int i=0;i<4;i++)if(r[i]!=i+1)ok=0;free(r);return ok?42:0;",
        "char*p=malloc(8);if(!p)return 1;*p=42;char*q=realloc(p,18446744073709551615ULL);int ok=q==0&&*p==42;free(p);return ok?42:0;",
        "free(0);int ok=malloc(0)==0&&calloc(0,8)==0&&calloc(8,0)==0&&realloc(0,0)==0;char*p=realloc(0,16);if(!p)return 1;ok=ok&&realloc(p,0)==0;return ok?42:0;",
        "return malloc(18446744073709551615ULL)==0&&calloc(9223372036854775808ULL,2)==0&&malloc(9223372036854710272ULL)==0?42:0;",
        "for(int i=0;i<100;i++){unsigned long long*p=malloc(8);if(!p)return 1;*p=i;int ok=*p==(unsigned long long)i;free(p);if(!ok)return 2;}return 42;",
    ] {
        qualifies(&format!("{prelude}int main(){{{body}}}"), 42);
    }
}

#[test]
fn indirect_pointer_updates_and_addresses_of_string_objects() {
    qualifies(
        "int main(){int a[2]={42,7};int*p=a;int**q=&p;int*old=(*q)++;*q-=1;return old==a&&p==a?*p:0;}",
        42,
    );
    qualifies(
        "int main(){char(*p)[4]=&\"abc\";return sizeof(*p)==4&&(*p)[1]=='b'?42:0;}",
        42,
    );
    qualifies(
        "char(*p)[4]=&\"abc\";int main(){return (*p)[2]=='c'?42:0;}",
        42,
    );
}

#[test]
fn composed_array_callbacks_and_integer_pointer_roundtrips() {
    qualifies(
        "int f(int x){return x+1;}int g(int x){return x-1;}int(*choose(int n))(int){return n?f:g;}int main(){return choose(1)(41);}",
        42,
    );
    qualifies(
        "unsigned long long strlen(const char*);unsigned long long(*lengths[1])(const char*)={strlen};int main(){return (int)lengths[0](\"abc\")+39;}",
        42,
    );
    qualifies(
        "int main(){int x=42;unsigned long long bits=(unsigned long long)&x;int*p=(int*)bits;return *p;}",
        42,
    );
    qualifies(
        "typedef int Row[2];Row a[2]={{10},12,20};int main(){return a[0][0]+a[0][1]+a[1][0]+a[1][1];}",
        42,
    );
}
