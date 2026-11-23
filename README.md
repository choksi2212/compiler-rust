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
| Push | Push a constant |
| Load / Store | Read / write a numbered local slot |
| Unary / Binary | Consume operands and push a result |
| Jump | Set the instruction pointer |
| Zero | Pop a condition and branch if zero |
| Print | Pop arguments, format output, push byte count |
| Halt | Finish execution |

Runtime errors include division by zero, integer overflow, uninitialized reads, invalid stack operations, and invalid instruction targets. Execution is limited to ten million VM instructions to bound accidental infinite loops. Program output is buffered until execution succeeds.

The compiler never invokes another C compiler. Tests check each compiler stage, errors, short-circuit logic, scope behavior, and all demo outputs with and without optimization.
