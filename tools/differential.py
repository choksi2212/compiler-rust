"""Optional development check; GCC is an oracle, never a runtime dependency."""
import pathlib
import random
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
COMPILER = ROOT / 'target/release/compiler.exe'
rng = random.Random(2212)

def run(args):
    return subprocess.run([str(a) for a in args], check=True, capture_output=True, timeout=10).stdout.replace(b'\r\n', b'\n')

with tempfile.TemporaryDirectory(prefix='compiler-differential-') as temp:
    folder = pathlib.Path(temp)
    for case in range(100):
        a, b, bound = rng.randint(-50, 50), rng.randint(1, 20), rng.randint(0, 15)
        op = rng.choice(['+', '-', '*', '/', '%', '<', '<=', '>', '>=', '==', '!=', '&&', '||'])
        text = f'''#include <stdio.h>
int main() {{
 int a={a}; int b={b}; int sum=0;
 for(int i=0;i<{bound};i=i+1) {{
   if(i%2==0) {{sum=sum+i;}} else {{sum=sum-1;}}
 }}
 {{int a=7;printf("%d ",a);}}
 printf("%d %d %d\\n",a{op}b,sum,!(a<b));
 return 0;
}}
'''
        source, executable = folder/'case.c', folder/'reference.exe'
        source.write_text(text)
        run(['gcc', '-std=c11', '-Wall', '-Werror', source, '-o', executable])
        expected = run([executable])
        for flags in [[], ['--no-opt']]:
            actual = run([COMPILER, source, *flags])
            assert actual == expected, (case, flags, text, actual, expected)
print('100 generated C programs matched GCC in both modes (200 comparisons). Seed: 2212.')
