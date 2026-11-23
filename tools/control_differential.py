"""P10 development oracle: defined C11 control-flow programs, GCC versus both backends/modes."""
from pathlib import Path
import hashlib
import json
import os
import random
import subprocess
import time

root = Path(__file__).resolve().parents[1]
folder = root / 'target/P10-oracle'
folder.mkdir(exist_ok=True)
summary_path = root / 'validation/P10-control-oracle.json'
started = time.monotonic()
cases, commands = [], []
summary_path.write_bytes(b'{"all_passed":false,"status":"running"}\n')


def digest(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def run(args, program=False, empty=False):
    env = os.environ.copy()
    if empty:
        env['PATH'] = ''
    p = subprocess.run([str(a) for a in args], cwd=folder, env=env, capture_output=True, timeout=15)
    commands.append(dict(command=[str(a) for a in args], exit=p.returncode,
                         empty_path=empty, stderr=p.stderr.decode('utf-8', errors='replace')))
    if not program and p.returncode:
        raise RuntimeError(commands[-1])
    return p.returncode & 0xffffffff, p.stdout.replace(b'\r\n', b'\n'), p.stderr


rng = random.Random(2212)
passed = False
try:
    run(['cargo', 'build', '--manifest-path', root / 'Cargo.toml', '--release', '--locked', '--offline'])
    compiler = root / 'target/release/compiler.exe'
    for i in range(100):
        n, a, b = rng.randint(1, 17), rng.randint(-8, 8), rng.randint(1, 7)
        source = f'''#include <stdio.h>
/* Owned compiler control-flow oracle {i}. All arithmetic is within int. */
int main(void){{
 int sum={a};int count=0;
 for(int j=0;j<{n};j++){{
   if(j==2)continue;
   switch(j%5){{case 0:sum+={b};case 1:sum+=j;break;
     case 2:{{int t=j+1;sum+=t;}}break;
     default:switch(j%2){{case 0:sum-=1;break;default:sum+=2;}}}}
   if(j>12)break;
 }}
 do{{count++;if(count==2)continue;sum+=count;}}while(count<4);
 int old=sum++;int fresh=++sum;sum-=3;sum*=2;sum/=2;sum%=97;
 int x=0;int y=0;x=y=3;
 for(;;){{x--;if(x==0)break;sum+=y;}}
 goto enter;
 {{int skipped=99;enter:skipped={b};sum+=skipped;goto leave;}}
 leave:;
 int k=0;again:k++;if(k<3)goto again;
 printf("%d %d %d %d %d %d %d %d\\n",sum,count,old,fresh,x,y,k,010+0x10+'A');
 printf("%d %s\\n",strlen_placeholder, "s");
 return {i % 29 - 7};
}}
'''
        # strlen is a compiler built-in but conventional C needs string.h,
        # whose preprocessing is a later phase. Use a first-NUL %s literal here.
        source = source.replace('printf("%d %s\\n",strlen_placeholder, "s");', 'printf("%s\\n", "nul\\0ignored");')
        path = folder / f'case{i:03}.c'
        path.write_bytes(source.encode('utf-8'))
        reference = folder / 'reference.exe'
        run(['gcc', '-std=c11', '-O0', '-Wall', '-Werror', path, '-o', reference])
        expected = run([reference], program=True)
        assert expected[2] == b''
        results = []
        for no_opt in [False, True]:
            flags = ['--no-opt'] if no_opt else []
            output = folder / f'case{i:03}-{no_opt}.exe'
            run([compiler, path, '-o', output, '--overwrite', *flags], empty=True)
            native = run([output], program=True, empty=True)
            vm = run([compiler, path, '--run-vm', *flags], program=True, empty=True)
            assert native == expected, (i, no_opt, 'native', native, expected, source)
            assert vm == expected, (i, no_opt, 'vm', vm, expected, source)
            results.append(dict(no_opt=no_opt, native_sha256=digest(output),
                                native_exit=native[0], vm_exit=vm[0]))
        cases.append(dict(case=i, source=source, source_sha256=digest(path),
                          expected_exit=expected[0], expected_stdout=expected[1].decode('utf-8'), results=results))
    passed = True
finally:
    summary = dict(all_passed=passed, seed=2212, programs_completed=len(cases),
                   native_comparisons=len(cases)*2, vm_comparisons=len(cases)*2,
                   normalization='CRLF to LF and Windows status as unsigned32 only',
                   elapsed_seconds=round(time.monotonic()-started, 3),
                   source_sha256={str(p.relative_to(root)):digest(p) for p in sorted((root/'src').glob('*.rs'))},
                   generator_sha256=digest(Path(__file__)))
    for name, value in [('P10-control-oracle-cases.json', cases), ('P10-control-oracle-commands.json', commands), ('P10-control-oracle.json', summary)]:
        (root/'validation'/name).write_bytes((json.dumps(value, indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}), flush=True)
