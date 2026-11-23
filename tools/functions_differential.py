"""P12 development oracle: LLP64 functions, variadics and storage versus the owned native backend."""
from pathlib import Path
import hashlib
import json
import os
import random
import subprocess
import time

root=Path(__file__).resolve().parents[1]
folder=root/'target/P12-oracle'
folder.mkdir(exist_ok=True)
summary_path=root/'validation/P12-functions-oracle.json'
summary_path.write_bytes(b'{"all_passed":false,"status":"running"}\n')
cases,commands=[],[]
started=time.monotonic()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args,program=False,empty=False):
    env=os.environ.copy()
    if empty:
        env['PATH']=''
    p=subprocess.run([str(a) for a in args],cwd=folder,env=env,capture_output=True,timeout=15)
    commands.append(dict(command=[str(a) for a in args],exit=p.returncode,empty_path=empty,
                         stderr=p.stderr.decode('utf-8',errors='replace')))
    if not program and p.returncode:
        raise RuntimeError(commands[-1])
    return p.returncode&0xffffffff,p.stdout.replace(b'\r\n',b'\n'),p.stderr


passed=False
rng=random.Random(2212)
try:
    target=run(['gcc','-dumpmachine'])[1].decode().strip()
    assert target=='x86_64-w64-mingw32',('oracle must match Windows LLP64',target)
    run(['cargo','build','--manifest-path',root/'Cargo.toml','--release','--locked','--offline'])
    binary=root/'target/release/compiler.exe'
    for case in range(100):
        count=case%13
        fixed=case%6+1
        params=','.join(f'long long p{i}' for i in range(count)) or 'void'
        expression='+'.join(f'p{i}*{i+1}LL' for i in range(count)) or '0'
        args=','.join(str((case+i)%29-14)+'LL' for i in range(count))
        named=','.join(f'int f{i}' for i in range(fixed))
        named_args=','.join(str(i) for i in range(fixed))
        variable_args=','.join(str(i*7-case) for i in range(count))
        comma=',' if count else ''
        source=f'''/* P12 defined-behavior ABI/storage/runtime differential {case}. */
int printf(const char*,...);
unsigned long long strlen(const char*);
static unsigned short guard=65530;
unsigned char byte=250;
int zero;
static int persistent(int x){{static int n=3;n+=x;return n;}}
long long weighted({params}){{return {expression};}}
int recurse(int n){{if(n<2)return 1;return n*recurse(n-1);}}
int odd(int);int even(int n){{if(n==0)return 1;return odd(n-1);}}
int odd(int n){{if(n==0)return 0;return even(n-1);}}
int variadic({named},...){{
 __builtin_va_list ap;__builtin_va_list copy;
 __builtin_va_start(ap,f{fixed-1});__builtin_va_copy(copy,ap);
 int sum=0;for(int i=0;i<{count};i++)sum+=__builtin_va_arg(ap,int);
 int again=0;for(int i=0;i<{count};i++)again+=__builtin_va_arg(copy,int);
 __builtin_va_end(copy);__builtin_va_end(ap);return sum+again;
}}
void update(void){{byte+=9;guard+=11;zero+=4;}}
int main(void){{
 update();int a=persistent({case%7});int b=persistent(2);
 long long w=weighted({args});int v=variadic({named_args}{comma}{variable_args});
 int r=recurse({case%7});int e=even({case%15});
 int n=printf("%d %d %d %d %d %d %d %d %s\\n",(int)w,v,r,e,a,b,(int)guard,(int)byte,"ok");
 return n+zero+(int)strlen("hello")+v+(int)w;
}}
'''
        path=folder/f'case{case:03}.c'
        path.write_bytes(source.encode('utf-8'))
        reference=folder/'reference.exe'
        run(['gcc','-std=c11','-pedantic-errors','-Wall','-Werror','-O0',path,'-o',reference])
        expected=run([reference],program=True)
        assert expected[2]==b''
        modes=[]
        for optimize in ['-O0','-O1']:
            output=folder/f'case{case:03}{optimize}.exe'
            run([binary,path,'--std=c11',optimize,'-o',output,'--overwrite'],empty=True)
            actual=run([output],program=True,empty=True)
            assert actual==expected,(case,optimize,actual,expected,source)
            modes.append(dict(optimization=optimize,exit=actual[0],sha256=digest(output)))
        cases.append(dict(case=case,source=source,source_sha256=digest(path),expected_exit=expected[0],expected_stdout=expected[1].decode('utf-8'),reference_sha256=digest(reference),modes=modes))
    passed=True
finally:
    summary=dict(all_passed=passed,programs_completed=len(cases),native_comparisons=len(cases)*2,
        seed=2212,oracle_target=locals().get('target'),elapsed_seconds=round(time.monotonic()-started,3),
        normalization='CRLF to LF and Windows process status as unsigned32 only',
        undefined_behavior='excluded; arithmetic stays in range, argument effects are sequenced, varargs types match',
        source_sha256={str(p.relative_to(root)):digest(p) for p in sorted((root/'src').glob('*.rs'))},
        generator_sha256=digest(Path(__file__)))
    for name,value in [('P12-functions-oracle.json',summary),('P12-functions-oracle-cases.json',cases),('P12-functions-oracle-commands.json',commands)]:
        (root/'validation'/name).write_bytes((json.dumps(value,indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
