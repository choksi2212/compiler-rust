"""Development-only P14 Windows LLP64 aggregate/layout/ABI differential oracle."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--count', type=int, default=104)
parser.add_argument('--skip-build', action='store_true')
parser.add_argument('--baseline-only', action='store_true', help='omit24 initializer/declaration extension cases')
args = parser.parse_args()
if not 1 <= args.count <= 104:
    parser.error('count must be 1..104 (104 covers all eight extents at all 13 positions)')
root = Path(__file__).resolve().parents[1]
folder = root / 'target/P14-oracle'
folder.mkdir(exist_ok=True)
summary_path = root / 'validation/P14-aggregate-oracle.json'
summary_path.write_bytes(b'{"all_passed":false,"status":"running"}\n')
cases, commands = [], []
started = time.monotonic()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_manifest():
    return {str(p.relative_to(root)): digest(p) for p in sorted((root / 'src').glob('*.rs'))}


def run(arguments, program=False, empty=False):
    arguments = [str(a) for a in arguments]
    env = os.environ.copy()
    if empty:
        env['PATH'] = ''
    result = subprocess.run(arguments, cwd=folder, env=env, capture_output=True, timeout=30)
    commands.append(dict(command=arguments, exit=result.returncode,
                         windows_exit=result.returncode & 0xffffffff, empty_path=empty,
                         stdout=result.stdout.decode('utf-8', errors='replace'),
                         stderr=result.stderr.decode('utf-8', errors='replace')))
    if not program and result.returncode:
        raise RuntimeError(commands[-1])
    return result.returncode & 0xffffffff, result.stdout.replace(b'\r\n', b'\n'), result.stderr


def source_for(case):
    extent = [1, 2, 4, 8, 3, 5, 12, 24][case % 8]
    position = case // 8 % 13
    bias = case % 19 + 1
    fixed = case % 6 + 1
    anonymous = case % 7
    scalars = [(f'int p{i}', str(i + bias)) for i in range(12)]
    parameters = [p for p, _ in scalars]
    arguments = [v for _, v in scalars]
    parameters.insert(position, 'struct R item')
    arguments.insert(position, 'src')
    prototype = ','.join(parameters)
    direct_args = ','.join(arguments)
    nested_args = arguments.copy()
    nested_args[position] = 'adjust(' + direct_args + ')'
    nested_args = ','.join(nested_args)
    fixed_params = ','.join(f'int f{i}' for i in range(fixed))
    fixed_sum = '+'.join(f'f{i}' for i in range(fixed))
    fixed_args = [str(i + 1) for i in range(fixed)]
    extra_args = [str(i + 3) for i in range(anonymous)] + ['src', '7']
    varargs = ','.join(fixed_args + extra_args)
    source = f'''/* P14 oracle {case}: payload {extent}, aggregate argument position {position}. */
int printf(const char*,...);
void*malloc(unsigned long long);
void free(void*);
struct R{{unsigned char bytes[{extent}];}};
struct Layout{{char a;int b;short c;long long d;}};
union U{{unsigned int n;unsigned char bytes[4];}};
struct Anonymous{{char tag;union{{int x;struct{{short y;short z;}};}};}};
struct Bits{{unsigned int a:3,b:5;signed int s:7;unsigned int c:30,d:4;unsigned char guard;}};
struct Separator{{unsigned char a:4;unsigned int:0;unsigned char last;}};
struct NoSeparator{{unsigned int:0;unsigned char last;}};
struct Mixed{{unsigned char a:4;unsigned short b:5;unsigned int c:6;unsigned char last;}};
struct Signed{{signed int s:3;}};
struct Flex{{int count;unsigned long long data[];}};
enum E{{NEG=-1,ZERO,POS=42}};
struct R adjust({prototype}){{
 int mix={'+'.join(f'p{i}' for i in range(12))};
 item.bytes[0]+=mix;item.bytes[{extent - 1}]^=90;
 return item;
}}
struct R variable({fixed_params},...){{
 __builtin_va_list ap;__builtin_va_start(ap,f{fixed - 1});
 int sum={fixed_sum};for(int i=0;i<{anonymous};i++)sum+=__builtin_va_arg(ap,int);
 struct R copy=__builtin_va_arg(ap,struct R);int tail=__builtin_va_arg(ap,int);
 __builtin_va_end(ap);copy.bytes[0]+=sum+tail;return copy;
}}
int main(void){{
 struct R src;for(int i=0;i<{extent};i++)src.bytes[i]=(unsigned char)(i*7+{bias});
 struct R (*callback)({prototype})=adjust;
 struct R direct=adjust({direct_args});
 struct R indirect=callback({direct_args});
 struct R nested=adjust({nested_args});
 struct R anonymous=variable({varargs});
 int payload=0;for(int i=0;i<{extent};i++){{
   if(src.bytes[i]!=(unsigned char)(i*7+{bias}))return 251;
   if(direct.bytes[i]!=indirect.bytes[i])return 252;
   payload+=direct.bytes[i]+nested.bytes[i]+anonymous.bytes[i];
 }}
 struct Layout layout;
 int offset_b=(int)((unsigned long long)&layout.b-(unsigned long long)&layout);
 int offset_d=(int)((unsigned long long)&layout.d-(unsigned long long)&layout);
 struct Anonymous named;named.x=0;named.y=11;named.z=13;
 union U u;u.n=0x12345678u;
 struct Bits bits;bits.a=0;bits.b={case % 32};bits.s={case % 90 - 45};bits.c={123456 + case * 11};bits.d={case % 16};bits.guard=91;
 int promoted=(bits.a-1)<0;bits.a=1;int old=bits.a++;int updated=++bits.d;bits.s+=1;
 int bit_score=bits.a+bits.b+bits.s+(int)bits.c+updated+bits.guard+promoted+old;
 unsigned char*representation=(unsigned char*)&bits;
 int first_byte=representation[0];
 struct Signed signed_field;int seven=7;int signed_assignment=(signed_field.s=seven);
 if(signed_assignment!=-1||signed_field.s!=-1)return 254;
 struct Separator separator;separator.a=7;separator.last=42;
 int sep_offset=(int)((unsigned long long)&separator.last-(unsigned long long)&separator);
 struct NoSeparator ignored;ignored.last=1;
 struct Mixed mixed;mixed.a=3;mixed.b=17;mixed.c=33;mixed.last=9;
 int mixed_offset=(int)((unsigned long long)&mixed.last-(unsigned long long)&mixed);
 struct Flex*flex=malloc(sizeof(struct Flex)+3*sizeof(unsigned long long));if(!flex)return 253;
 flex->count=3;for(int i=0;i<3;i++)flex->data[i]=i+3;
 int flexible=0;for(int i=0;i<flex->count;i++)flexible+=(int)flex->data[i];free(flex);
 enum E enumeration=POS;
 int size_score=(int)sizeof(struct R)+(int)sizeof(struct Layout)+(int)_Alignof(struct Layout)+(int)sizeof(struct Bits)+(int)sizeof(struct Separator)+(int)_Alignof(struct Separator)+(int)sizeof(struct NoSeparator)+(int)_Alignof(struct NoSeparator)+(int)sizeof(struct Mixed)+(int)_Alignof(struct Mixed)+(int)sizeof(struct Flex)+(int)_Alignof(struct Flex);
 int score=payload+bit_score+size_score+offset_b+offset_d+named.y+named.z+u.bytes[0]+u.bytes[3]+separator.last+ignored.last+mixed.a+mixed.b+mixed.c+mixed.last+flexible+enumeration+NEG+first_byte+sep_offset+mixed_offset;
 int count=printf("%d %d %d %d %d\\n",score,payload,bit_score,size_score,first_byte);
 count+=printf("%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\\n",(int)sizeof(struct R),(int)sizeof(struct Layout),(int)_Alignof(struct Layout),offset_b,offset_d,(int)sizeof(struct Bits),(int)sizeof(struct Separator),(int)_Alignof(struct Separator),sep_offset,(int)sizeof(struct NoSeparator),(int)_Alignof(struct NoSeparator),(int)sizeof(struct Mixed),(int)_Alignof(struct Mixed),mixed_offset,(int)sizeof(struct Flex),(int)_Alignof(struct Flex));
 return score+count;
}}
'''
    return source, extent, position, fixed, anonymous


def feature_source(case):
    bias = case + 1
    index = case % 4
    source = f'''/* P14 initializer/declaration extension {case}. */
int printf(const char*,...);
int identity(int);
struct Inner{{int x;short y;}};
struct Item{{char tag;struct Inner pair;int values[4];const char*text;}};
struct Reference{{int*p;int n;}};
struct Operations{{int(*fn)(int);}};
union Choice{{int small;long long large;}};
struct Flags{{unsigned int a:3;signed int sign:4;unsigned int b:5;}};
struct Aligned{{char lead;_Alignas(16) int n;char last;}};
struct Assertion{{_Static_assert(sizeof(int)==4,"target int");int n;}};
_Static_assert(sizeof(struct Aligned)==32,"aligned record");
_Static_assert(sizeof(struct Flags)==4,"bitfield storage");
int global=42;
struct Reference reference={{.p=&global,.n={bias}}};
struct Reference*static_literal=&(struct Reference){{.p=&global,.n=7}};
struct Operations operations={{.fn=identity}};
struct Item static_item={{.values[{index}]={bias},.tag=3,.pair.y=12,.pair.x=10,.text="static"}};
union Choice choice={{.large=1234}};
struct Flags flags={{.a=7,.b=19,.sign=-2}};
int scattered[]={{[4]=20,[1]=10,12}};
unsigned char prefix_global;
_Alignas(16) struct Item aligned_global;
int calls;
int identity(int n){{return n;}}
struct Item produce(void){{calls++;return static_item;}}
struct Aligned adjust(struct Aligned value){{value.n+=2;return value;}}
int static_alignment(void){{_Alignas(16) static unsigned char bytes[3];bytes[0]=1;return ((unsigned long long)bytes%16)==0;}}
int main(void){{
 _Static_assert(_Alignof(struct Aligned)==16,"record alignment");
 struct Item local={{.tag={bias},.pair={{.y=12,.x=10}},.values={{[3]=20,[0]=10,12}},.text="local"}};
 struct Item copy=local;local.pair.x=1;
 union Choice local_choice={{.small=17}};
 struct Flags local_flags={{.sign=-3,.b=11,.a=5}};
 unsigned char prefix_local;prefix_local=1;prefix_global=1;
 _Alignas(16) struct Item aligned_local;
 _Alignas(struct Aligned) unsigned char aligned_bytes[3];
 struct Aligned aligned={{.n=40}};
 struct Aligned adjusted=adjust(aligned);
 int alignment=((unsigned long long)&aligned_global%16)==0&&((unsigned long long)&aligned_local%16)==0&&((unsigned long long)aligned_bytes%16)==0&&static_alignment();
 struct Reference*first=0;struct Reference*again=0;int i=0;
 repeat:
 again=&(struct Reference){{.p=&global,.n=i}};
 if(i==0)first=again;else if(first!=again)return 221;
 i++;if(i<3)goto repeat;
 int*scalar=&(int){{42}};*scalar+=1;
 int(*array)[3]=&(int[3]){{10,12,20}};
 int counter=0;int selected=_Generic(counter++,int:7,default:99);
 int unselected=_Generic(0,int:1,default:counter++);
 _Generic(0,int:counter,default:global)=3;
 const int constant=1;
 int qualifier=_Generic(constant,int:5,default:9);
 int array_type=_Generic(scattered,int*:6,default:9);
 int function_type=_Generic(identity,int(*)(int):8,default:9);
 int record_type=_Generic(produce(),struct Item:13,default:0);
 int copy_value=_Generic(0,int:copy,default:local).pair.x;
 if(counter!=3||calls!=0||first->n!=2||adjusted.n!=42||aligned.n!=40)return 222;
 if(sizeof(scattered)!=20||scattered[0]!=0||scattered[2]!=12||scattered[4]!=20)return 223;
 if(copy.values[2]!=0||copy.values[0]!=10||copy.values[1]!=12||copy.values[3]!=20)return 224;
 if(flags.a!=7||flags.sign!=-2||flags.b!=19||local_flags.sign!=-3)return 225;
 int score=*reference.p+reference.n+*static_literal->p+static_literal->n+operations.fn(5)+static_item.tag+static_item.pair.x+static_item.pair.y+static_item.values[{index}]+(int)choice.large+copy.tag+copy.pair.x+copy.pair.y+copy.values[0]+copy.values[1]+copy.values[3]+local_choice.small+local_flags.a+local_flags.b+local_flags.sign+alignment+first->n+*scalar+(*array)[0]+(*array)[1]+(*array)[2]+selected+unselected+qualifier+array_type+function_type+record_type+copy_value+adjusted.n+prefix_local+prefix_global;
 int count=printf("%d %s %d %d %d\\n",score,copy.text,alignment,first->n,record_type);
 return score+count;
}}
'''
    return source, None, None, None, None


passed = False
try:
    target = run(['gcc', '-dumpmachine'])[1].decode().strip()
    assert target == 'x86_64-w64-mingw32', ('oracle must match Windows LLP64', target)
    gcc_version = run(['gcc', '--version'])[1].decode().splitlines()[0]
    before_build = source_manifest()
    if not args.skip_build:
        run(['cargo', 'build', '--manifest-path', root / 'Cargo.toml', '--release', '--locked', '--offline'])
    binary = root / 'target/release/compiler.exe'
    compiler_hash = digest(binary)
    compiled_sources = source_manifest()
    build_sources_unchanged = before_build == compiled_sources
    total = args.count + (0 if args.baseline_only else 24)
    for case in range(total):
        extension = case >= args.count
        source, extent, position, fixed, anonymous = feature_source(case-args.count) if extension else source_for(case)
        path = folder / f'case{case:03}.c'
        path.write_bytes(source.encode('utf-8'))
        reference = folder / f'reference{case:03}.exe'
        run(['gcc', '-std=c11', '-pedantic-errors', '-Wall', '-Werror', '-O0', path, '-o', reference])
        expected = run([reference], program=True)
        assert expected[2] == b''
        modes = []
        for optimize in ['-O0', '-O1']:
            output = folder / f'case{case:03}{optimize}.exe'
            run([binary, path, '--std=c11', optimize, '-o', output, '--overwrite'], empty=True)
            actual = run([output], program=True, empty=True)
            assert actual == expected, (case, optimize, actual, expected, source)
            modes.append(dict(optimization=optimize, exit=actual[0], stdout=actual[1].decode(),
                              sha256=digest(output)))
        cases.append(dict(case=case, family='initializer/declaration' if extension else 'ABI/layout', payload_bytes=extent, aggregate_position=position,
                          named_variadic_parameters=fixed, anonymous_scalars_before_record=anonymous,
                          source=source, source_sha256=digest(path), expected_exit=expected[0],
                          expected_stdout=expected[1].decode(), reference_sha256=digest(reference), modes=modes))
    passed = True
finally:
    stable_sources=locals().get('compiled_sources')==source_manifest()
    complete_contract=args.count==104 and not args.baseline_only
    qualified=passed and complete_contract and not args.skip_build and locals().get('build_sources_unchanged',False) and stable_sources
    summary = dict(all_passed=qualified, comparisons_passed=passed,
                   execution_mode='prebuilt baseline; binary source unverified' if args.skip_build else 'rebuilt current source',
                   complete_contract=complete_contract,
                   requested_programs=args.count+(0 if args.baseline_only else 24), programs_completed=len(cases),
                   initializer_declaration_cases=0 if args.baseline_only else 24,
                   native_comparisons=len(cases)*2, oracle_target=locals().get('target'),
                   gcc_version=locals().get('gcc_version'), compiler_sha256=locals().get('compiler_hash'),
                   source_sha256=locals().get('compiled_sources',{}) if not args.skip_build else {},
                   observed_working_source_sha256=locals().get('compiled_sources',{}),
                   working_sources_unchanged=locals().get('compiled_sources') == source_manifest(),
                   build_sources_unchanged=locals().get('build_sources_unchanged', False),
                   generator_sha256=digest(Path(__file__)), elapsed_seconds=round(time.monotonic()-started, 3),
                   matrix='eight payload extents1/2/4/8/3/5/12/24, aggregate positions0..12; full matrix requires104 cases',
                   normalization='CRLF to LF and Windows process status as unsigned32 only',
                   undefined_behavior='excluded: no padding comparison, no signed overflow, correct varargs types, sequenced effects',
                   target_choices='Windows LLP64 little-endian; unsigned pointer-address conversion for member offsets; negative enumerator fixes signed-int enum comparison; signed bitfield narrowing follows the frozen low-bit target policy')
    for name, value in [('P14-aggregate-oracle.json', summary), ('P14-aggregate-oracle-cases.json', cases), ('P14-aggregate-oracle-commands.json', commands)]:
        (root/'validation'/name).write_bytes((json.dumps(value,indent=2)+'\n').encode('utf-8'))
print(json.dumps({k:v for k,v in summary.items() if not k.endswith('sha256')}),flush=True)
