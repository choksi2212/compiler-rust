# C compiler in Rust

A dependency-free educational compiler for the C subset defined in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). That document is the single source of truth for scope, implementation sequence, and change approval.

```powershell
cargo build
cargo run -- examples/pattern.c
cargo run -- examples/addition.c --tokens --ast --ir --optimized-ir --bytecode
cargo test
```

The compiler performs lexical analysis, parsing, semantic checks, three-address code generation, optimization, and stack bytecode generation. Its virtual machine executes the resulting bytecode. Stage dumps go to stderr and program output goes to stdout. Use `--no-opt` to execute without optimization.

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
compiler program.c
compiler program.c --ir --optimized-ir --bytecode
```

This compiles and executes the supported C program immediately. It does not write a native executable or support GCC's `-o` option. Native executable generation would require a separate approved change to the output target.

To install from this repository on another Windows machine with Rust installed:

```powershell
git clone https://github.com/choksi2212/compiler-rust.git
cd compiler-rust
cargo install --path .
compiler examples/addition.c
```

Ensure Cargo's `bin` directory is on PATH. Alternatively, `cargo build --release` produces `target\release\compiler.exe`; copy it to a folder on PATH or invoke it by its full path. The compiled compiler executable does not need Rust or GCC installed to run supported C programs.

## Aggressive validation

`cargo test` includes arithmetic matrices, 10,000 deterministic random malformed inputs, source mutation, nested control flow, shadowing, fresh loop-local declarations, checked arithmetic errors, and oversized syntax rejection. Parser recursion is limited to 64 levels and each expression tail to 128 binary operators to reject pathological input cleanly.

An optional GCC comparison suite is available for development:

```powershell
cargo build --release
python tools/differential.py
```

It requires Python and GCC and compares 100 generated, defined-behavior C programs with optimized and unoptimized execution. GCC is only a test oracle. Leading-zero nondecimal literals and unsupported compound operators are rejected. Reserved C keywords and the built-in names `main`, `printf`, and `strlen` cannot be declared as local variables.
