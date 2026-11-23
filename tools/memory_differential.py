"""P13 development oracle: LLP64 object memory, callbacks and owned allocation versus the owned native backend."""
from pathlib import Path
import hashlib
import json
import os
import random
import subprocess
import time

root=Path(__file__).resolve().parents[1]
folder=root/'target/P13-oracle'
folder.mkdir(exist_ok=True)
summary_path=root/'validation/P13-memory-oracle.json'
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
        count=case%9+3
        bias=case%37
        extent=count*2+5
        source=f'''/* P13 defined-behavior native object/allocator oracle {case}. */
int printf(const char*,...);
unsigned long long strlen(const char*);
int strcmp(const char*,const char*);
char*strcpy(char*,const char*);
void*memcpy(void*,const void*,unsigned long long);
void*memmove(void*,const void*,unsigned long long);
void*memset(void*,int,unsigned long long);
void*malloc(unsigned long long);
void*calloc(unsigned long long,unsigned long long);
void*realloc(void*,unsigned long long);
void free(void*);
int global[2][3]={{{{{bias},2,3}},{{4,5,6}}}};
int*middle=&global[1][1];
int plus(int n){{return n+{bias};}}
int minus(int n){{return n-{bias};}}
int (*functions[2])(int)={{plus,minus}};
int apply(int(*f)(int),int x){{return f(x);}}
int main(void){{
 int a[{count}];for(int i=0;i<{count};i++)a[i]=i*3+{bias};
 int*p=a;int*q=a+{count};long long distance=q-p;
 int sum=0;for(int i=0;i<{count};i++)sum+=p[i];
 unsigned char bytes[8];memset(bytes,0x1234,8);
 char text[16];strcpy(text,"abcdef");memmove(text+2,text,5);text[7]=0;
 char copy[16]={{0}};memcpy(copy,text,8);int same=strcmp(copy,text)==0;
 int callback=apply(functions[{case%2}],sum);
 int*heap=calloc({extent},sizeof(int));if(!heap)return 111;
 int zeros=1;for(int i=0;i<{extent};i++)if(heap[i])zeros=0;
 for(int i=0;i<{count};i++)heap[i]=a[i];
 int*grown=realloc(heap,({extent}+7)*sizeof(int));if(!grown){{free(heap);return 112;}}
 int equal=1;for(int i=0;i<{count};i++)if(grown[i]!=a[i])equal=0;
 grown[{extent}+6]=42;int last=grown[{extent}+6];free(grown);
 unsigned long long*other=malloc(16);if(!other)return 113;
 other[0]=0xffffffffffffffffULL;other[1]=other[0]+2;int wrapped=(int)other[1];free(other);
 int score=sum+callback+(int)distance+*middle+bytes[7]+same+zeros+equal+last+wrapped;
 int n=printf("%d %s\\n",score,copy);
 return score+n+(int)strlen(copy)+(q>p);
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
    for name,value in [('P13-memory-oracle.json',summary),('P13-memory-oracle-cases.json',cases),('P13-memory-oracle-commands.json',commands)]:
        (root/'validation'/name).write_bytes((json.dumps(value,indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
