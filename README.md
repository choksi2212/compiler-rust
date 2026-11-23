# C compiler in Rust

A dependency-free educational compiler for the C subset defined in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). That document is the single source of truth for scope, implementation sequence, and change approval.

```powershell
cargo build
cargo run -- examples/pattern.c -o pattern.exe
.\pattern.exe
cargo run -- examples/pattern.c --run-vm
cargo run -- examples/addition.c --tokens --ast --ir --optimized-ir --bytecode
cargo test
```

The compiler performs lexical analysis, parsing, semantic checks, three-address code generation and optimization. With `-o`, its owned x64 encoder, Windows ABI lowering, linker, PE writer and runtime produce a standalone Windows x64 executable. Without `-o`, its stack bytecode generator and VM execute the program immediately. Stage dumps go to stderr. Use `--no-opt` to disable IR optimization.

Six examples and their expected output are in `examples/`: addition, loop, factorial, pattern, conditional, and strings.

The current subset supports one `int main()` or `int main(void)`, integer locals/expressions/returns, assignment and arithmetic compound assignment, prefix/postfix `++`/`--`, `if`, `while`, `for` (including empty clauses), `do/while`, `break`, `continue`, `switch` with constant cases/fallthrough, labels and `goto`. Falling off main returns zero. `//` and `/* */` comments, decimal/octal/hex values fitting signed32, ASCII character constants, and simple/octal/hex ASCII escapes are supported. Wider/unsigned/suffixed literals remain explicit errors until the type-system phase; bitwise/shift operations are not implemented yet.

It accepts a leading `#include <stdio.h>` directive; this is not a preprocessor. `printf` and `strlen` remain built-ins. Strings are UTF-8 literals; escaped NUL terminates C string operations, and `strlen` counts bytes before that terminator. General pointers, mutable strings, arrays, input functions, other headers and user-defined functions are future phases. Integer operations use checked signed32 arithmetic. Uninitialized reads report a runtime error; jumping into a new scope clears its locals' initialization state rather than reusing a previous lifetime's values.

For the partial typed C11 path, use an explicit mode:

```powershell
compiler examples/typed_integers.c --std=c11 -O0 -o typed.exe
.\typed.exe
$LASTEXITCODE  # 0 means all checks in this example passed
compiler examples/typed_integers.c --std=c11 -fsyntax-only --ast
```

This path implements Windows LLP64 integers: signed plain char8, short16, int/long32 and long long64, plus their unsigned variants and `_Bool`. It supports typed literals/suffixes, casts, promotions, arithmetic/bitwise/shift operations, typedefs, qualifiers, target `sizeof`/`_Alignof`, integer locals and the documented control flow. Unsigned arithmetic wraps at its type width. Signed overflow, invalid shifts and uninitialized reads are checked extensions; the `--std=c11` flag selects evolving language rules and does **not** claim conformance.

Typed output currently uses main's process result: general calls, printf/strlen calls, global storage, pointer/array execution, floating types and preprocessing are future phases. Pointer/array/function declaration records and layout queries are implemented; accepting a type declaration for `-fsyntax-only` does not imply its runtime operations are supported. Keep using the default teaching path for the original printf demos. Typed mode requires `-o` or `-fsyntax-only` and rejects VM flags. `-fsyntax-only` allows token/AST dumps and rejects output/IR-dump combinations. The native publisher's overwrite/input-protection rules apply to both paths.

Current resource limits include source16MiB, one million tokens, syntax recursion64,128 operators across a complete expression, type derivation128 and4096 typed basic blocks. These are partial-compiler limits and remain part of the later conformance audit. Signed narrowing uses the declared low-bit two's-complement policy and signed right shifts are arithmetic.

Bytecode instruction reference:

| Instruction | Behavior |
| --- | --- |
| Reset | Clear a local slot at declaration |
| Push | Push a constant |
| Load / Store | Read / write a numbered local slot |
| Unary / Binary | Consume operands and push a result |
| Jump | Set the instruction pointer |
| Zero | Pop a condition and branch if zero |
| Print | Pop arguments, format output, push byte count |
| Halt / Return | Finish with zero / computed integer process result |

Runtime errors include division by zero, integer overflow, uninitialized reads, invalid stack operations, and invalid instruction targets. VM execution is limited to ten million instructions to bound accidental infinite loops. Program output is buffered until execution succeeds.

The compiler never invokes another C compiler. Tests check each compiler stage, errors, short-circuit logic, scope behavior, and all demo outputs with and without optimization.

## Install and use as a command

On this computer, `compiler.exe` is installed in `C:\Users\niklaus\.cargo\bin`, which is already on PATH. Open PowerShell in the folder containing your C file and run:

```powershell
compiler program.c -o program.exe
.\program.exe
compiler program.c --run-vm --ir --optimized-ir --bytecode
compiler program.c -o program.exe --overwrite --emit=machine
compiler --print-capabilities
```

With `-o`, compilation writes the executable and does not run it. Run `.\program.exe` separately. Native executables contain the owned runtime and import only the documented Windows OS services; they do not need the compiler, Rust, GCC, the C source or a project DLL at execution time. The compiler itself remains a Rust bootstrap executable.

An existing destination requires `--overwrite`. Input files and the running compiler are protected, including hard-link aliases. Output is validated, written to a unique temporary file beside the destination and published using a Windows move operation; a failed compile preserves the previous file. The parent directory must exist. Use `--` before an input name starting with a dash. Source files must be UTF-8 and at most 16MiB; the token budget is one million.

`--tokens`, `--ast`, `--ir`, and `--optimized-ir` work in both modes. `--emit=machine` prints linked native instruction bytes and requires `-o`; `--bytecode` and `--run-vm` conflict with native output. Exit codes are 0 for success, 1 for compilation/IO/runtime failure, and 2 for invalid command usage. Generated programs use runtime status codes 1–7 for checked arithmetic, initialization, output/memory/IO errors.

To install from this repository on another Windows machine with Rust installed:

```powershell
git clone https://github.com/choksi2212/compiler-rust.git
cd compiler-rust
cargo install --path .
compiler examples/addition.c
```

Ensure Cargo's `bin` directory is on PATH. Alternatively, `cargo build --release` produces `target\release\compiler.exe`; copy it to a folder on PATH or invoke it by its full path. The Rust/MSVC bootstrap compiler does not need Rust or GCC at execution time, but its inspected imports require the Visual C++ x64 runtime (`VCRUNTIME140.dll`) and Windows Universal CRT/API-set services. The package declares this prerequisite and does not redistribute Microsoft DLLs. Generated C executables have no VCRUNTIME/UCRT dependency. Empty-PATH checks on this host are not a clean-machine or all-Windows-version certification.

## Aggressive validation

`cargo test` includes arithmetic matrices, 10,000 deterministic random malformed inputs, source mutation, nested control flow, shadowing, fresh loop-local declarations, checked arithmetic errors, and oversized syntax rejection. Parser recursion is limited to 64 levels and each expression tail to 128 binary operators to reject pathological input cleanly.

An optional GCC comparison suite is available for development:

```powershell
cargo build --release
python tools/differential.py
```

It requires Python and GCC and compares 100 generated, defined-behavior C programs with optimized and unoptimized execution. GCC is only a test oracle. Octal and hexadecimal literals use C radix rules; typed suffixes, wide values and bitwise/shift operations are rejected until their semantics are implemented. Reserved C keywords and the built-in names `main`, `printf`, and `strlen` cannot be declared as local variables.

## Native backend development status

Version 0.2.0 compiles the documented teaching C subset to real Windows x64 PE32+ executables. Generated programs use owned checked integer arithmetic, short-circuit control flow, pooled UTF-8 literals, `%d`/`%s` formatting, `strlen`, buffered output, Windows stack frames, unwind metadata and ASLR relocations. Native output is limited to 16MiB by default; unlike the VM, native loops have no instruction quota. Production compilation does not invoke an external C compiler, assembler, linker or C runtime. Full C11 and the full standard library are future phases; current capability and evidence are tracked in the canonical plan.

P09 distribution evidence covers its frozen subset release; P10 adds the documented control flow. P11 introduces the explicit typed path above. Run its gate with `python validation/run_gate.py P11`; independent development comparisons are `python tools/typed_differential.py` and `python tools/encoding_differential.py --phase P11`. The P10 control-flow oracle remains `python tools/control_differential.py`. Optional independent encoder/native-source comparisons use GNU tools only as development oracles. Evidence is in `validation/`. The portable Windows package is assembled from the tested release binary under `target/P09-package/`; no official release upload is performed automatically.
