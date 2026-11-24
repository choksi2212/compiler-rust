"""P14 development-only GCC oracle for declaration and generic semantics."""
from pathlib import Path
import hashlib
import json
import os
import subprocess
import time

root=Path(__file__).resolve().parents[1]
folder=root/'target/P14-declaration-oracle'
folder.mkdir(exist_ok=True)
summary_path=root/'validation/P14-declaration-oracle.json'
summary_path.write_bytes(b'{"all_passed":false,"status":"running"}\n')
commands=[]
records=[]
started=time.monotonic()

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def source_manifest():
    return {str(p.relative_to(root)):digest(p) for p in sorted((root/'src').glob('*.rs'))}

manifest=source_manifest()

def run(args,empty=False):
    env=os.environ.copy()
    if empty: env['PATH']=''
    result=subprocess.run([str(a) for a in args],cwd=folder,env=env,capture_output=True,timeout=60)
    record=dict(command=[str(a) for a in args],exit=result.returncode&0xffffffff,
                stdout=result.stdout.decode('utf-8',errors='replace').replace('\r\n','\n'),
                stderr=result.stderr.decode('utf-8',errors='replace'),empty_path=empty)
    commands.append(record)
    return record

valid=[
('generic-control-effect','int main(void){int count=0;int n=_Generic(count++,int:42,default:0);return count==0?n:0;}'),
('generic-unselected-effect','int main(void){int count=0;int n=_Generic(1,int:42,default:count++);return count==0?n:0;}'),
('generic-unselected-fault','int main(void){return _Generic(1,int:42,default:1/0);}'),
('generic-lvalue','int main(void){int x=1,y=2;_Generic(0,int:x,default:y)=42;return x;}'),
('generic-const','int main(void){const int x=7;return _Generic(x,int:42,const int:1,default:0);}'),
('generic-char','int main(void){unsigned char x=1;return _Generic(x,unsigned char:42,int:0,default:1);}'),
('generic-array','int main(void){int a[3];return _Generic(a,int*:42,int[3]:1,default:0);}'),
('generic-function','int f(int n){return n;}int main(void){return _Generic(f,int(*)(int):42,default:0);}'),
('generic-dead-control','int f(void);int main(void){return _Generic(f(),int:42,default:0);}'),
('assertions','_Static_assert(sizeof(long)==4,"LLP64");struct S{_Static_assert(sizeof(int)==4,"width");int x;};int main(void){_Static_assert(sizeof(struct S)==4,"record");return 42;}'),
('aligned-objects','char leading;_Alignas(16) int g;int main(void){char first=1;_Alignas(16) int x=41;return ((unsigned long long)&g%16)==0&&((unsigned long long)&x%16)==0?first+x:0;}'),
('aligned-members','struct S{char first;_Alignas(16) int x;char last;};int main(void){struct S s;s.x=42;return sizeof(struct S)==32&&_Alignof(struct S)==16&&((unsigned long long)&s.x%16)==0?s.x:0;}'),
('alignment-position','int main(void){char first=1;int _Alignas(16) x=41;return ((unsigned long long)&x%16)==0?first+x:0;}'),
('alignment-type','int main(void){_Alignas(long long) char bytes[8];return ((unsigned long long)bytes%8)==0?42:0;}'),
('alignment-zero','int main(void){_Alignas(0) int x=42;return x;}'),
('alignment-strongest','int main(void){_Alignas(8) _Alignas(16) int x=42;return ((unsigned long long)&x%16)==0?x:0;}'),
]
invalid=[
('assert-false','_Static_assert(0,"failure");int main(void){return 0;}'),
('assert-nonconstant','int n;_Static_assert(n,"not constant");int main(void){return 0;}'),
('generic-duplicate','typedef int T;int main(void){return _Generic(0,int:1,T:2);}'),
('generic-two-defaults','int main(void){return _Generic(0,default:1,default:2);}'),
('generic-no-match','int main(void){return _Generic(0,char:1);}'),
('generic-invalid-dead-arm','int main(void){return _Generic(0,int:42,default:missing);}'),
('weak-pointer-alignment','int main(void){_Alignas(4) int*p;return 0;}'),
('zero-register-alignment','int main(void){_Alignas(0) register int x;return 0;}'),
('zero-function-alignment','_Alignas(0) int f(void){return 0;}int main(void){return f();}'),
('zero-bitfield-alignment','struct S{_Alignas(0) int x:3;};int main(void){return 0;}'),
]
passed=False
differences=[]
try:
    target=run(['gcc','-dumpmachine'])
    assert target['exit']==0 and target['stdout'].strip()=='x86_64-w64-mingw32',target
    build=run(['cargo','build','--manifest-path',root/'Cargo.toml','--release','--locked','--offline'])
    assert build['exit']==0,build
    compiler=root/'target/release/compiler.exe'
    for name,source in valid:
        path=folder/(name+'.c');path.write_bytes(source.encode('utf-8'))
        reference=folder/(name+'-gcc.exe')
        built=run(['gcc','-std=c11','-pedantic-errors','-O0',path,'-o',reference]);assert built['exit']==0,built
        expected=run([reference]);assert expected['exit']==42 and not expected['stdout'] and not expected['stderr'],expected
        modes=[]
        for optimization in ['-O0','-O1']:
            output=folder/(name+optimization+'.exe')
            built=run([compiler,path,'--std=c11',optimization,'-o',output,'--overwrite'],empty=True);assert built['exit']==0,built
            actual=run([output],empty=True)
            assert (actual['exit'],actual['stdout'],actual['stderr'])==(expected['exit'],expected['stdout'],expected['stderr']),(name,actual,expected)
            modes.append(dict(optimization=optimization,exit=actual['exit'],image_sha256=digest(output)))
        records.append(dict(case=name,source=source,source_sha256=digest(path),valid=True,expected_exit=42,reference_sha256=digest(reference),modes=modes))
    for name,source in invalid:
        path=folder/(name+'.c');path.write_bytes(source.encode('utf-8'))
        reference=run(['gcc','-std=c11','-pedantic-errors','-fsyntax-only',path]);assert reference['exit']!=0,reference
        actual=run([compiler,path,'--std=c11','-fsyntax-only'],empty=True);assert actual['exit']==1 and actual['stderr'],actual
        records.append(dict(case=name,source=source,source_sha256=digest(path),valid=False,gcc_exit=reference['exit'],native_exit=actual['exit']))
    source='_Alignas(16) int x;_Alignas(8) int x;int main(void){return x;}'
    path=folder/'alignment-policy-difference.c';path.write_bytes(source.encode('utf-8'))
    reference=run(['gcc','-std=c11','-pedantic-errors','-fsyntax-only',path])
    actual=run([compiler,path,'--std=c11','-fsyntax-only'],empty=True)
    assert reference['exit']==0 and actual['exit']==1,(reference,actual)
    differences.append(dict(case='conflicting explicit alignment declarations',source=source,source_sha256=digest(path),gcc_exit=reference['exit'],compiler_exit=actual['exit'],policy='owned target rejects unequal explicit redeclaration alignments; GCC accepts, excluded from agreement counts'))
    assert manifest==source_manifest(),'compiler source changed during oracle execution; rerun after edits settle'
    passed=True
finally:
    summary=dict(all_passed=passed,valid_programs=sum(r['valid'] for r in records),native_comparisons=sum(r['valid'] for r in records)*2,
        invalid_programs=sum(not r['valid'] for r in records),elapsed_seconds=round(time.monotonic()-started,3),
        oracle_target='x86_64-w64-mingw32',source_sha256=manifest,generator_sha256=digest(Path(__file__)),
        normalization='CRLF to LF and process result as Windows unsigned32',
        declared_oracle_differences=differences,
        scope='C11 declaration/generic semantics; published implementation-specific bitfield generic controlling-type choice excluded')
    for name,data in [('P14-declaration-oracle.json',summary),('P14-declaration-oracle-cases.json',records),('P14-declaration-oracle-commands.json',commands)]:
        (root/'validation'/name).write_bytes((json.dumps(data,indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
