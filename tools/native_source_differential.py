"""Optional native source oracle. Foreign tools are development-only."""
from pathlib import Path
import subprocess,os,json,time,hashlib
root=Path(__file__).resolve().parents[1]
started=time.monotonic();logs=[];commands=[];passed=False
summary_path=root/'validation/P08-native-oracle.json'
summary_path.write_bytes(b'{"all_passed":false,"status":"running"}\n')
def run(args,empty_path=False):
 env=os.environ.copy()
 if empty_path:env['PATH']=''
 p=subprocess.run([str(a) for a in args],capture_output=True,cwd=root,env=env,timeout=15)
 commands.append(dict(command=[str(a) for a in args],exit=p.returncode,empty_path=empty_path))
 if p.returncode:raise RuntimeError((args,p.returncode,p.stdout,p.stderr))
 return p.stdout.replace(b'\r\n',b'\n')
try:
 print(run(['cargo','run','--quiet','--locked','--offline','--example','native_oracle_corpus']).decode(),flush=True)
 folder=root/'target/P08-oracle'
 for case in range(100):
  source=folder/f'case{case:03}.c';reference=folder/'reference.exe'
  run(['gcc','-std=c11','-Wall','-Werror',source,'-o',reference]);expected=run([reference]);actuals=[];hashes=[]
  for mode in ['false','true']:
   executable=folder/f'case{case:03}-{mode}.exe';actual=run([executable],empty_path=True)
   assert actual==expected,(case,mode,source.read_text(encoding='utf-8'),actual,expected)
   actuals.append(actual.decode());hashes.append(hashlib.sha256(executable.read_bytes()).hexdigest())
  logs.append(dict(case=case,source=source.read_text(encoding='utf-8'),expected_stdout=expected.decode(),native_stdout=actuals,native_sha256=hashes,passed=True))
 passed=True
finally:
 summary=dict(seed=2212,programs_completed=len(logs),native_comparisons=len(logs)*2,all_passed=passed,normalization='CRLF to LF only',native_path_environment='',elapsed_seconds=round(time.monotonic()-started,3),source_sha256={str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((root/'src').glob('*.rs'))},generator_sha256=hashlib.sha256((root/'examples/native_oracle_corpus.rs').read_bytes()).hexdigest())
 (root/'validation/P08-native-oracle-cases.json').write_bytes((json.dumps(logs,indent=2)+'\n').encode())
 (root/'validation/P08-native-oracle-commands.json').write_bytes((json.dumps(commands,indent=2)+'\n').encode())
 summary_path.write_bytes((json.dumps(summary,indent=2)+'\n').encode())
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
