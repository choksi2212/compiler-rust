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

Use explicit loop updates such as `i = i + 1`. This version requires a condition in a `for` loop and supports one `int main()` ending in `return 0;`. It accepts the leading `#include <stdio.h>` directive; it does not implement a preprocessor. `printf` and `strlen` are built-ins. Strings are literals, and `strlen` counts their UTF-8 bytes. General pointers, mutable strings, arrays, input functions, comments, increment operators, other headers, and user-defined functions are outside this version. Integer operations use checked signed 32-bit arithmetic. Reading an uninitialized variable reports a runtime error.

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
| Halt | Finish execution |

Runtime errors include division by zero, integer overflow, uninitialized reads, invalid stack operations, and invalid instruction targets. Execution is limited to ten million VM instructions to bound accidental infinite loops. Program output is buffered until execution succeeds.

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

It requires Python and GCC and compares 100 generated, defined-behavior C programs with optimized and unoptimized execution. GCC is only a test oracle. Leading-zero nondecimal literals and unsupported compound operators are rejected. Reserved C keywords and the built-in names `main`, `printf`, and `strlen` cannot be declared as local variables.

## Native backend development status

Version 0.2.0 compiles the documented teaching C subset to real Windows x64 PE32+ executables. Generated programs use owned checked integer arithmetic, short-circuit control flow, pooled UTF-8 literals, `%d`/`%s` formatting, `strlen`, buffered output, Windows stack frames, unwind metadata and ASLR relocations. Native output is limited to 16MiB by default; unlike the VM, native loops have no instruction quota. Production compilation does not invoke an external C compiler, assembler, linker or C runtime. Full C11 and the full standard library are future phases; current capability and evidence are tracked in the canonical plan.

Run the phase gate with `python validation/run_gate.py P09`. Optional independent encoder/native-source comparisons use GNU tools only as development oracles. Evidence is in `validation/`. The portable Windows package is assembled from the tested release binary under `target/P09-package/`; no official release upload is performed automatically.
