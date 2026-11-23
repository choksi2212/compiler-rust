"""P11 development oracle: LLP64 C11 integer programs versus the owned native backend."""
from pathlib import Path
import hashlib
import json
import os
import random
import subprocess
import time

root=Path(__file__).resolve().parents[1]
folder=root/'target/P11-oracle'
folder.mkdir(exist_ok=True)
summary_path=root/'validation/P11-typed-oracle.json'
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
        seed=rng.getrandbits(64)
        multiplier=rng.randrange(1,400)
        divisor=rng.randrange(1,200)
        shift=rng.randrange(1,63)
        iterations=rng.randrange(1,20)
        negative=rng.randrange(1,100)
        source=f'''/* P11 oracle {case}; defined arithmetic and documented low-bit int conversion. */
typedef unsigned long long U;
typedef int Matrix[3][4];
typedef int (*Callback)(int);
int main(void){{
 volatile U state=0x{seed:016x}ULL;
 const U factor={multiplier}ULL;
 unsigned int u=4294967295u-{negative}u;
 long s=-{negative}L;
 unsigned long mix=u+s;
 long long widened=u+(long long)s;
 unsigned char byte=250;byte+=21;
 short narrow=(short){seed%30000};
 unsigned short us=(unsigned short){seed%65536};
 int promoted=narrow+us;
 _Bool truth=state;
 for(unsigned int i=0;i<{iterations};i++){{
   if(i==2)continue;
   switch(i%4){{case 0:state+=factor;case 1:state^=i;break;
     case 2:state-=factor;break;default:state*=factor;}}
   state=(state<<{shift})^(state>>{64-shift});
 }}
 U quotient=state/{divisor}ULL;
 U remainder=state%{divisor}ULL;
 U selected=truth ? quotient : remainder;
 int untouched=1;
 U layout=sizeof(Matrix)+sizeof(Callback)+sizeof(long)+sizeof(long long)+sizeof(void*)+_Alignof(short);
 U sizes=sizeof(byte++)+sizeof(+byte)+sizeof(++untouched);
 if(untouched!=1)return 123;
 U score=(U)promoted+(U)mix+(U)widened+(U)byte+layout+sizes;
 return (int)(state^(state>>32)^selected^remainder^score);
}}
'''
        path=folder/f'case{case:03}.c'
        path.write_bytes(source.encode('utf-8'))
        reference=folder/'reference.exe'
        run(['gcc','-std=c11','-pedantic-errors','-Wall','-Werror','-O0',path,'-o',reference])
        expected=run([reference],program=True)
        assert expected[1:]==(b'',b'')
        modes=[]
        for optimize in ['-O0','-O1']:
            output=folder/f'case{case:03}{optimize}.exe'
            run([binary,path,'--std=c11',optimize,'-o',output,'--overwrite'],empty=True)
            actual=run([output],program=True,empty=True)
            assert actual==expected,(case,optimize,actual,expected,source)
            modes.append(dict(optimization=optimize,exit=actual[0],sha256=digest(output)))
        cases.append(dict(case=case,source=source,source_sha256=digest(path),expected_exit=expected[0],modes=modes))
    passed=True
finally:
    summary=dict(all_passed=passed,programs_completed=len(cases),native_comparisons=len(cases)*2,
        seed=2212,oracle_target=locals().get('target'),elapsed_seconds=round(time.monotonic()-started,3),
        normalization='CRLF to LF and Windows process status as unsigned32 only',
        undefined_behavior='excluded; final unsigned-to-signed int conversion follows documented low-bit target policy',
        source_sha256={str(p.relative_to(root)):digest(p) for p in sorted((root/'src').glob('*.rs'))},
        generator_sha256=digest(Path(__file__)))
    for name,value in [('P11-typed-oracle.json',summary),('P11-typed-oracle-cases.json',cases),('P11-typed-oracle-commands.json',commands)]:
        (root/'validation'/name).write_bytes((json.dumps(value,indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
