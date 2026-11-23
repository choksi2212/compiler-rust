"""Record phase test commands and actual outcomes; never advances phase state."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('phase')
args = parser.parse_args()
if not re.fullmatch(r'P[0-9]{2}', args.phase):
    parser.error('expected phase such as P01')
root = Path(__file__).resolve().parents[1]
commands = [
    ['cargo', 'fmt', '--check'],
    ['cargo', 'clippy', '--all-targets', '--locked', '--offline', '--', '-D', 'warnings'],
    ['cargo', 'test', '--all-targets', '--locked', '--offline'],
    ['cargo', 'test', '--all-targets', '--release', '--locked', '--offline'],
]
log = root / 'validation' / (args.phase + '-gate.log')
result_path = root / 'validation' / (args.phase + '-gate.json')
records = []
with log.open('w', encoding='utf-8') as stream:
    stream.write('Phase gate evidence; phase status is governed by IMPLEMENTATION_PLAN.md.\n')
    stream.write('Baseline HEAD: ' + subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True))
    for command in commands:
        print('RUN:', subprocess.list2cmdline(command), flush=True)
        stream.write('\nCOMMAND: ' + subprocess.list2cmdline(command) + '\n')
        try:
            result = subprocess.run(command, cwd=root, stdout=subprocess.PIPE,
                                    stderr=subprocess.STDOUT, timeout=60)
            output = result.stdout.decode('utf-8', errors='replace').replace('\r\n', '\n')
            code = result.returncode
        except subprocess.TimeoutExpired as error:
            output = (error.stdout or b'').decode('utf-8', errors='replace') + '\nTIMEOUT\n'
            code = None
        stream.write(output + '\nEXIT: ' + str(code) + '\n')
        stream.flush()
        print(output, flush=True)
        records.append(dict(command=command, exit=code, passed=code == 0))
        if code != 0:
            break
files = sorted([*root.glob('src/**/*.rs'), *root.glob('tests/**/*.rs'), root/'Cargo.toml', root/'Cargo.lock'])
manifest = {str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
summary = dict(phase=args.phase, commands=records, source_sha256=manifest,
               required_commands_executed=len(records) == len(commands),
               commands_passed=len(records) == len(commands) and all(r['passed'] for r in records))
result_path.write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
raise SystemExit(0 if summary['commands_passed'] else 1)
