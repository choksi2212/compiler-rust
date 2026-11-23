"""Compare our encoder bytes with GNU as; development-only, never production."""
from pathlib import Path
import subprocess,tempfile,json,hashlib
root=Path(__file__).resolve().parents[1]
fixtures=root/'target/encoding-fixtures.tsv'
def run(args):
 p=subprocess.run([str(a) for a in args],cwd=root,capture_output=True,timeout=60)
 if p.returncode:raise RuntimeError((args,p.returncode,p.stdout,p.stderr))
 return p.stdout
print(run(['cargo','run','--quiet','--locked','--offline','--example','encoding_oracle','--',fixtures]).decode(),flush=True)
records=[line.split('\t',1) for line in fixtures.read_text().splitlines()]
expected=b''.join(bytes.fromhex(h) for h,_ in records)
with tempfile.TemporaryDirectory(prefix='x64 independent oracle ') as temp:
 folder=Path(temp);assembly=folder/'fixtures.s';obj=folder/'fixtures.obj';binary=folder/'fixtures.bin'
 assembly.write_text('.text\n'+ '\n'.join(asm for _,asm in records)+'\n.byte 0xde,0xad,0xbe,0xef,0x11,0x22,0x33,0x44\n')
 run(['as','--64',assembly,'-o',obj]);run(['objcopy','-O','binary','-j','.text',obj,binary])
 actual=binary.read_bytes();marker=bytes.fromhex('deadbeef11223344')
 # Different valid instruction encodings must not be confused with a semantic
 # mismatch. Independently disassemble both complete streams with GNU objdump.
 marker_at=actual.rfind(marker)
 assert marker_at>=0,'assembler end marker missing'
 reference=actual[:marker_at]
 own_path=folder/'own.bin';reference_path=folder/'reference.bin'
 own_path.write_bytes(expected);reference_path.write_bytes(reference)
 def decode(path):
  import re
  text=run(['objdump','-D','-b','binary','-m','i386:x86-64','--no-show-raw-insn',path]).decode()
  rows=[]
  for line in text.splitlines():
   match=re.match(r'^\s*([0-9a-f]+):\s+(.+)$',line)
   if match:rows.append((int(match.group(1),16),' '.join(match.group(2).split())))
  assert rows,'no instructions disassembled'
  return rows
 own_rows=decode(own_path);reference_rows=decode(reference_path)
 assert len(own_rows)==len(reference_rows),(len(own_rows),len(reference_rows))
 alternatives=[]
 for i,((at,own_text),(ref_at,ref_text)) in enumerate(zip(own_rows,reference_rows)):
  assert own_text==ref_text,(i,own_text,ref_text)
  own_end=own_rows[i+1][0] if i+1<len(own_rows) else len(expected)
  ref_end=reference_rows[i+1][0] if i+1<len(reference_rows) else len(reference)
  if expected[at:own_end]!=reference[ref_at:ref_end]:
   alternatives.append(dict(instruction=own_text,own=expected[at:own_end].hex(),reference=reference[ref_at:ref_end].hex()))
summary=dict(fixtures_sha256=hashlib.sha256(fixtures.read_bytes()).hexdigest(),generator_sha256=hashlib.sha256((root/'examples/encoding_oracle.rs').read_bytes()).hexdigest(),fixtures=len(records),own_bytes=len(expected),reference_bytes=len(reference),decoded_instructions=len(own_rows),all_decoded_operations_match=True,valid_alternate_encodings=alternatives,oracle='GNU as, objcopy and objdump; development only')
(root/'validation/P03-encoding-oracle.json').write_bytes((json.dumps(summary,indent=2)+'\n').encode())
print(json.dumps(summary),flush=True)
