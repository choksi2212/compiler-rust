"""Package and qualify the gated Windows binary. Development-only, not a compiler dependency."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / 'validation/P09-distribution.json'
report = {'passed': False, 'commands': [], 'phase': 'P09'}


def save():
    EVIDENCE.write_bytes((json.dumps(report, indent=2, ensure_ascii=True) + '\n').encode('utf-8'))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args, cwd, empty_path=True):
    env = os.environ.copy()
    if empty_path:
        env['PATH'] = ''
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, capture_output=True, timeout=30)
    record = {'args': [str(a) for a in args], 'cwd': str(cwd), 'empty_path': empty_path,
              'exit': result.returncode, 'stdout': result.stdout.decode('utf-8'),
              'stderr': result.stderr.decode('utf-8')}
    report['commands'].append(record)
    save()
    if result.returncode:
        raise RuntimeError(record)
    return result


save()
gate = json.loads((ROOT / 'validation/P09-gate.json').read_text(encoding='utf-8'))
assert gate['commands_passed']
assert all(digest(ROOT / p) == h for p, h in gate['source_sha256'].items())
release = ROOT / 'target/release/compiler.exe'
installed = Path.home() / '.cargo/bin/compiler.exe'
assert digest(release) == digest(installed), 'installed binary must match gated release'
package = ROOT / 'target/P09-package'
package.mkdir(exist_ok=True)
shutil.copy2(release, package / 'compiler.exe')
shutil.copy2(ROOT / 'README.md', package / 'README.md')
examples = package / 'examples'
examples.mkdir(exist_ok=True)
for name in ['addition', 'loop', 'factorial', 'pattern', 'conditional', 'strings']:
    for suffix in ['.c', '.expected']:
        shutil.copy2(ROOT / f'examples/{name}{suffix}', examples / f'{name}{suffix}')

# Inspect the actual bootstrap imports and document its existing MSVC dependency.
bootstrap = run(['objdump', '-p', release], ROOT, empty_path=False).stdout.decode('utf-8')
report['compiler_imports'] = re.findall(r'DLL Name: ([^\r\n]+)', bootstrap)
report['compiler_sha256'] = digest(release)
prerequisites = '''Windows x64 teaching C compiler 0.2.0

Extract this directory. In PowerShell:
  .\\compiler.exe examples\\pattern.c -o pattern.exe
  .\\pattern.exe
Existing output: add --overwrite. Full C11 is not implemented.

The Rust/MSVC bootstrap compiler uses the Visual C++ x64 runtime
(VCRUNTIME140.dll) and Windows Universal CRT/API-set services. These must
already be installed; this package does not redistribute Microsoft DLLs.
It does not require Rust, Cargo, GCC or the repository at execution time.
Generated C executables use only Windows OS services and the project-owned
runtime; they have no VCRUNTIME/UCRT/compiler/source dependency.

Validated on this Windows host with an empty PATH; not a clean-VM or
all-Windows-version certification. See README.md for limits and options.
'''
(package / 'START-HERE.txt').write_bytes(prerequisites.encode('utf-8'))
with tempfile.TemporaryDirectory(prefix='compiler installed qualification ') as tmp:
    outside = Path(tmp)
    for binary in [installed, package / 'compiler.exe']:
        assert b'0.2.0' in run([binary, '--version'], outside).stdout
        for name in ['addition', 'loop', 'factorial', 'pattern', 'conditional', 'strings']:
            source = outside / 'source.c'
            source.write_bytes((examples / f'{name}.c').read_bytes())
            output = outside / 'program.exe'
            run([binary, source, '-o', output, '--overwrite'], outside)
            source.unlink()
            actual = run([output], outside).stdout.replace(b'\r\n', b'\n')
            expected = (examples / f'{name}.expected').read_bytes().replace(b'\r\n', b'\n')
            assert actual == expected, name
            imports = run(['objdump', '-p', output], ROOT, empty_path=False).stdout.decode('utf-8')
            dlls = re.findall(r'DLL Name: ([^\r\n]+)', imports)
            assert [d.lower() for d in dlls] == ['kernel32.dll'], dlls
            report.setdefault('generated_images', []).append({'name': name, 'binary': str(binary),
                'sha256': digest(output), 'dlls': dlls})
            output.unlink()
report['files'] = {str(p.relative_to(package)): digest(p) for p in sorted(package.rglob('*')) if p.is_file()}
archive = ROOT / 'target/compiler-0.2.0-windows-x64.zip'
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as stream:
    for name in report['files']:
        stream.write(package / name, f'compiler-0.2.0-windows-x64/{name}')
with zipfile.ZipFile(archive) as stream:
    assert stream.testzip() is None
report.update(passed=True, archive=str(archive), archive_sha256=digest(archive),
              source_sha256=gate['source_sha256'])
save()
print(json.dumps({'passed': True, 'commands': len(report['commands']),
                  'archive': str(archive), 'sha256': report['archive_sha256']}))
