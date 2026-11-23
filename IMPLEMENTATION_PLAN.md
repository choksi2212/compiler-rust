# C Subset Compiler — Implementation Plan

This document is the single source of truth for the project’s scope and implementation sequence. Implementation must follow this plan. If a change would alter the language subset, compiler architecture, output target, or milestones below, ask the user first and update this document only after approval.

## Project objective

Build an educational compiler, implemented in Rust, that accepts a clearly defined subset of C and compiles supported programs into bytecode for a small virtual machine. The project will demonstrate the compiler pipeline from source text through execution, with diagnostics at each relevant stage.

This is a compiler for the subset defined here, not a general-purpose C compiler and not a wrapper around GCC or Clang.

## Fixed architecture

```text
C source
  → lexer
  → parser and abstract syntax tree
  → semantic analysis
  → three-address intermediate representation
  → basic optimizations
  → custom stack-machine bytecode
  → bytecode virtual machine
```

- **Compiler implementation language:** Rust.
- **Input language:** the C subset specified below.
- **Output and execution target:** project-owned stack-machine bytecode and VM. Generated programs must not depend on GCC/Clang to execute.
- **Command-line interface:** `compiler <source.c>` compiles and runs a supported program; stage-output options will expose tokens, AST, intermediate code, optimized code, and bytecode for learning and viva demonstrations.
- **Dependencies:** prefer Rust standard library; any additional dependency must be justified in this document before adoption.

## Supported C subset

The first complete version will support one `int main()` function and:

- `int` variables, declarations with optional initializers, assignment, and lexical block scopes.
- Integer literals, parentheses, unary minus, and `+`, `-`, `*`, `/`, `%` expressions with C-like precedence.
- Comparisons `<`, `<=`, `>`, `>=`, `==`, `!=`; logical `&&`, `||`, and `!`.
- `if` / `else`, `while`, and `for` statements; nested blocks.
- `printf` with string literals and the `%d` and `%s` conversions, plus `\n` and `\t` escapes. Arguments must match the conversions.
- String literals as immutable values and the built-in `strlen("literal")` operation. General C pointers, arrays, and mutable strings are outside the initial subset.
- `return 0;` at the end of `main`.
- A leading `#include <stdio.h>` may be accepted as a conventional header directive; it does not provide the implementation of `printf`, which is built into the VM.

Unsupported C syntax must produce a source-located, understandable diagnostic rather than silently generating incorrect output. The compiler will not claim support for arbitrary C programs.

## Demo programs

The project will include six small `.c` examples using only the supported subset:

1. Integer addition and formatted output.
2. A `for` loop that computes a sum.
3. A `while` loop that computes a factorial.
4. Nested loops that print a star or number pattern.
5. A conditional program such as checking whether a number is even or odd.
6. String output and `strlen` on a string literal.

Each example will include its expected output so the compiler’s behavior can be demonstrated and checked consistently.

## Implementation sequence

### Stage 0 — Repository and project skeleton

Establish the Rust crate in the supplied repository, with source modules and an `examples/` directory. Keep this plan as the only project-plan document; concise usage instructions may later go in the README, but scope and sequencing remain governed here.

### Stage 1 — Lexer

Recognize identifiers, keywords, integer literals, string literals, punctuation, and supported operators. Track line and column positions. Report invalid characters and unterminated strings.

### Stage 2 — Parser and AST

Parse the supported `main` function, declarations, statements, and expressions. Use precedence-aware expression parsing. Report syntax errors with source locations. Add parser demonstrations for each syntax construct before later stages depend on them.

### Stage 3 — Semantic analysis

Track nested scopes; reject duplicate declarations in a scope, undeclared variable uses, invalid assignments, and invalid `printf` argument/conversion combinations. Validate the supported `main` shape and return statement.

### Stage 4 — Intermediate representation

Lower the AST to three-address code with explicit temporaries and labels for branches and loops. Provide a readable IR dump for viva explanation.

### Stage 5 — Basic optimization

Implement constant folding and simple constant propagation, then remove unreachable instructions where safely possible. Show both pre- and post-optimization IR. Optimization must preserve behavior.

### Stage 6 — Bytecode generation and virtual machine

Translate optimized IR into documented stack-machine instructions. Implement integer arithmetic, comparisons, jumps, variables, string constants, formatted output, and `strlen`. Add runtime diagnostics for division by zero and invalid bytecode states.

### Stage 7 — End-to-end examples and documentation

Complete all six C examples, expected outputs, CLI help, and a concise explanation of the pipeline and supported C subset. Verify each demo end to end and check representative syntax and semantic errors.

## Completion criteria

- The compiler is implemented in Rust and builds as a command-line program.
- All six listed C examples compile and produce their documented output on the project VM.
- The CLI can display each major compiler representation needed for a viva walkthrough.
- Unsupported syntax and semantic errors produce useful source-located diagnostics.
- The implementation does not invoke GCC/Clang or another C compiler to compile or run the input.
- The supported language and limitations are documented accurately.

## Change control

This plan governs implementation. Do not begin code implementation until the user has reviewed this plan. If implementation reveals that a different feature, architecture, dependency, or target is needed, stop before making that change, explain the specific issue, and ask the user. Once approved, update this plan first; then implement the approved plan.

## Implementation status and validation

User approved implementation on 2026-11-23, requiring testing after each stage and real compiler logic throughout. The selected execution target remains the custom bytecode VM specified above.

| Stage | Status | Validation before the following stage |
| --- | --- | --- |
| 0: repository and crate | Complete | Cargo check/build succeeded; origin points to the supplied repository |
| 1: lexer | Complete | Five tests passed for positions, operators, strings, headers, and invalid input |
| 2: parser | Complete | Construct, precedence, and syntax-error tests passed; total eight tests |
| 3: semantics | Complete | Valid scope/format and rejection tests passed; total ten tests |
| 4: IR | Complete | Loop lowering and branch/label structure test passed; total eleven tests |
| 5: optimizer | Complete | Constant folding, dead instructions, error preservation, and control-flow boundary tests passed; total thirteen tests |
| 6: bytecode and VM | Complete | Loop/scope equivalence, short circuit, strings, uninitialized reads, division by zero, invalid stack and jump tests passed; total seventeen tests |
| 7: examples and CLI | Complete | Six demo outputs matched in both optimized and unoptimized runs; CLI stage dumps and failures tested |

Implementation details within the approved subset: integer operations are checked signed 32-bit operations; loop increments use explicit assignment; `for` conditions are required. Bytecode execution has a ten-million-instruction limit, and output is buffered. `printf` returns the emitted byte count. See README for usage and bytecode instruction descriptions; it introduces no separate project plan.
