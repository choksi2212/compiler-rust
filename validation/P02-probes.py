"""Reproduce P02 release CLI probes; evidence only, not compiler implementation."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
EXE = ROOT / 'target/release/compiler.exe'
CASES = [
    ('addition', 'int a=25;int b=17;printf("Sum = %d\\n",a+b);', 'Sum = 42\n'),
    ('factorial', 'int n=6;int r=1;while(n>1){r=r*n;n=n-1;}printf("%d",r);', '720'),
    ('pattern', 'for(int r=1;r<=3;r=r+1){for(int c=1;c<=r;c=c+1){printf("*");}printf("\\n");}', '*\n**\n***\n'),
    ('skipped-divide', 'printf("%d %d",0&&(1/0),1||(1/0));', '0 1'),
    ('printf-count', 'int n=printf("abc");printf("%d",n);', 'abc3'),
    ('string-length', 'printf("%s %d","hello",strlen("hello"));', 'hello 5'),
    ('scope-loop', 'int x=4;for(int i=0;i<3;i=i+1){int x=i;printf("%d",x);}printf("%d",x);', '0124'),
    ('precedence', 'printf("%d %d",2+3*4,20/5/2);', '14 2'),
]
ERRORS = [
    ('undeclared', 'x=3;', 'undeclared'),
    ('duplicate', 'int x=1;int x=2;', 'duplicate'),
    ('bad-format', 'printf("%d","text");', 'mismatch'),
    ('divide-zero', 'printf("%d",1/0);', 'division by zero'),
    ('overflow', 'printf("%d",2147483647+1);', 'overflow'),
    ('self-init', 'int x=1;{int x=x;printf("%d",x);}', 'uninitialized'),
    ('fresh-loop', 'for(int i=0;i<2;i=i+1){int x;if(i==0){x=9;}else{printf("%d",x);}}', 'uninitialized'),
    ('bad-escape', 'printf("\\q");', 'escape'),
    ('missing-semicolon', 'int x=1', 'expected'),
]
records = []

def invoke(label, source, flags, cwd, expected=None, error=None, empty_path=False):
    args = [str(EXE), str(source), *flags]
    env = os.environ.copy()
    if empty_path:
        env['PATH'] = ''
    result = subprocess.run(args, capture_output=True, timeout=10, cwd=cwd, env=env)
    out = result.stdout.decode('utf-8', errors='replace').replace('\r\n', '\n')
    err = result.stderr.decode('utf-8', errors='replace').replace('\r\n', '\n')
    passed = (result.returncode == 0 and out == expected) if error is None else (
        result.returncode == 1 and error in err.lower() and out == '')
    record = dict(case=label, command=args, cwd=str(cwd), empty_path=empty_path,
                  input_source=source.read_text() if source.exists() else None,
                  expected_stdout=expected, expected_error_contains=error,
                  exit=result.returncode, stdout=out, stderr=err, passed=passed)
    records.append(record)
    assert passed, record

try:
    with tempfile.TemporaryDirectory(prefix='P02 release probes ') as directory:
        folder = Path(directory)
        for label, body, expected in CASES:
            source = folder / (label + ' program.c')
            source.write_text('int main(){' + body + 'return 0;}')
            for flags in [[], ['--no-opt']]:
                invoke(label + str(flags), source, flags, folder, expected=expected)
        for label, body, error in ERRORS:
            source = folder / (label + '.c')
            source.write_text('int main(){' + body + 'return 0;}')
            for flags in [[], ['--no-opt']]:
                invoke(label + str(flags), source, flags, folder, error=error)
        source = folder / 'standalone demo.c'
        source.write_text('int main(){printf("standalone");return 0;}')
        invoke('outside repository with empty PATH', source, [], folder,
               expected='standalone', empty_path=True)
        invoke('missing-source', folder / 'not-present.c', [], folder, error='cannot find')
finally:
    (ROOT / 'validation/P02-probes.json').write_text(
        json.dumps(records, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')
    (ROOT / 'validation/P02-probes.log').write_text(
        'COMMAND: python validation/P02-probes.py\n' +
        '\n'.join(json.dumps(r, ensure_ascii=False) for r in records) + '\n', encoding='utf-8')
print('Release process probes passed:', len(records))
