# C Subset Compiler â€” Implementation Plan

This document is the single source of truth for the projectâ€™s scope and implementation sequence. Implementation must follow this plan. If a change would alter the language subset, compiler architecture, output target, or milestones below, ask the user first and update this document only after approval.

## Project objective

Build an educational compiler, implemented in Rust, that accepts a clearly defined subset of C and compiles supported programs into bytecode for a small virtual machine. The project will demonstrate the compiler pipeline from source text through execution, with diagnostics at each relevant stage.

This is a compiler for the subset defined here, not a general-purpose C compiler and not a wrapper around GCC or Clang.

## Fixed architecture

```text
C source
  â†’ lexer
  â†’ parser and abstract syntax tree
  â†’ semantic analysis
  â†’ three-address intermediate representation
  â†’ basic optimizations
  â†’ custom stack-machine bytecode
  â†’ bytecode virtual machine
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

Each example will include its expected output so the compilerâ€™s behavior can be demonstrated and checked consistently.

## Implementation sequence

### Stage 0 â€” Repository and project skeleton

Establish the Rust crate in the supplied repository, with source modules and an `examples/` directory. Keep this plan as the only project-plan document; concise usage instructions may later go in the README, but scope and sequencing remain governed here.

### Stage 1 â€” Lexer

Recognize identifiers, keywords, integer literals, string literals, punctuation, and supported operators. Track line and column positions. Report invalid characters and unterminated strings.

### Stage 2 â€” Parser and AST

Parse the supported `main` function, declarations, statements, and expressions. Use precedence-aware expression parsing. Report syntax errors with source locations. Add parser demonstrations for each syntax construct before later stages depend on them.

### Stage 3 â€” Semantic analysis

Track nested scopes; reject duplicate declarations in a scope, undeclared variable uses, invalid assignments, and invalid `printf` argument/conversion combinations. Validate the supported `main` shape and return statement.

### Stage 4 â€” Intermediate representation

Lower the AST to three-address code with explicit temporaries and labels for branches and loops. Provide a readable IR dump for viva explanation.

### Stage 5 â€” Basic optimization

Implement constant folding and simple constant propagation, then remove unreachable instructions where safely possible. Show both pre- and post-optimization IR. Optimization must preserve behavior.

### Stage 6 â€” Bytecode generation and virtual machine

Translate optimized IR into documented stack-machine instructions. Implement integer arithmetic, comparisons, jumps, variables, string constants, formatted output, and `strlen`. Add runtime diagnostics for division by zero and invalid bytecode states.

### Stage 7 â€” End-to-end examples and documentation

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

### Aggressive validation and command installation â€” 2026-11-23

The user requested aggressive testing followed by command-line installation. Within the approved compiler pipeline, regressions were added and fixes made for fresh loop-local lifetime, self-initialization under shadowing, reserved names, invalid declaration positions, and unsupported compound/nondecimal tokens. Reset instructions now clear declaration slots at each execution; optimizer knowledge is invalidated for reset and print result writes. Parser resource bounds reject pathological nesting and long expression chains.

Validation: 29 automated tests pass, including 16,150 arithmetic executions across both optimization modes, 10,000 deterministic random input attempts, source mutations, oversized syntax, and all six demos. The optional deterministic GCC differential suite passed 100 generated programs in both modes (200 comparisons). Formatting and Clippy checks pass. This evidence covers the tested cases; it is not proof of correctness for all input programs.

Installed the release executable with `cargo install --path .` and verified the `compiler` command from outside the repository. Installation, examples, and the optional differential suite are documented in README. The compiler still targets the approved bytecode VM. Native `.exe` output for compiled C input requires approval before implementation.


## Native compiler expansion — user-directed architecture

The user rejected external compiler/linker dependence and directed us to build the complete toolchain ourselves. This section supersedes the earlier VM-only output target for future deliveries. The existing VM remains a tested reference backend during migration. The scope is broader C support through a project-owned native backend, not a claim of current C conformance.

### Target and ownership

Proposed language target: hosted C11 on Windows x64. Language version is a working target for specification, not a certification. We own parsing, semantic analysis, preprocessing, intermediate code, optimization, machine instruction encoding, symbol resolution, PE executable writing, and the C library/runtime implementation. No GCC, Clang, external assembler, linker, third-party C compiler, or bytecode interpreter may generate or execute the native output. Rust's normal build toolchain builds our compiler itself. Generated programs may use Windows OS services such as process exit, file IO, and virtual memory; implementing Windows is outside this compiler project.

Full acceptance of valid C11 programs also requires the standard library and correct target-specific semantics. Nonstandard compiler extensions, other platforms, and programs requiring unrelated third-party libraries cannot be covered by a promise of accepting any source code.

### Ordered native milestones and test gates

- **8A: PE executable foundation.** Implement PE32+ headers, section placement, imports, and entrypoint bytes in Rust. Emit and launch genuine x64 executables that invoke the Windows ExitProcess API. Validate headers, section bounds, import resolution, and process exit statuses. No frontend/native CLI integration is claimed at this stage.
- **8B: machine code backend.** Encode the current IR directly into x64 instruction bytes, including locals, arithmetic, comparisons, jumps, checked errors, and correct Windows calling convention. Build a project-owned native runtime for integer/string output and length using OS IO services. Compare actual native execution against both VM modes for the complete current demo set; no precomputed output or VM embedding.
- **8C: native CLI and distribution.** Implement `compiler program.c -o program.exe`, output-path protection, useful build diagnostics, and standalone execution checks. Preserve stage inspection. Test failure cleanup, paths containing spaces, runtime errors, and execution from another directory without compiler availability.
- **9: conventional syntax and control flow.** Comments, character literals, integer bases/suffixes, increment/decrement, compound assignments, flexible returns, break/continue, do/while and switch. Define each new operation before code and test precedence, evaluation count, and control flow.
- **10: functions and types.** Function declarations/definitions, parameters, calls, recursion, stack frames, integer types, conversions, globals and storage duration. Test C-defined evaluation rules and ABI behavior.
- **11: memory.** Arrays, pointers, address/dereference, mutable strings, sizeof, object lifetime, allocation and deallocation. Own the runtime implementation; use Windows memory services underneath it. Establish layout and lifetime rules before coding.
- **12: aggregates and floating point.** Structs, unions, enums, qualifiers, typedefs, bitfields, floating types, alignment and target layout.
- **13: preprocessing and translation units.** Macros, conditional inclusion, headers, source locations across includes, internal/external linkage, object format and project-owned linking.
- **14: hosted C11 conformance work.** Build and test the standard library, remaining C11 features, and a conformance matrix. Document unsupported cases explicitly until independently tested. Language and runtime coverage must be measured rather than inferred from example count.

Each milestone must pass focused tests and all earlier regressions before the next. Detailed grammar, ABI, library and diagnostic rules must be added here before the corresponding implementation. Routine implementation decisions within these milestones are authorized. Changes to architecture, dependencies or agreed target still require asking the user first. Implementation status must distinguish foundation work from working end-to-end C compilation.

### Immediate acceptance gate

Complete and test 8A before starting 8B. Completion of 8A does not mean the C compiler can yet emit arbitrary native programs. Native CLI is delivered only after the entire existing subset has a tested native backend.

### Native foundation validation

8A implemented in `src/native_pe.rs`: project-owned PE32+ headers, aligned executable and import sections, a Kernel32 import table, and native entrypoint payload packaging. Windows process tests executed generated machine code with exit statuses 0, 37 and 255, including paths containing spaces. Empty/oversized payloads and section bounds are tested. The foundation currently permits at most 4096 code bytes, uses a fixed image base without base relocations, and imports only ExitProcess. These are foundation constraints to replace as the native backend grows, not claims of general executable support. No native C CLI option is implemented yet. 8B and subsequent milestones remain outstanding.
