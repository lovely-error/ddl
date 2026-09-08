# DDL (Dataflow Description Language)

[![CI](https://github.com/balcovt/ddl/actions/workflows/ci.yml/badge.svg)](https://github.com/balcovt/ddl/actions)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

DDL is a compiler that translates high-level dataflow specifications into synthesizable Verilog-2005.

## Why DDL?
DDL came from dissatisfaction with existing methods of creating designs for FPGAs. You can read more details [here](docs/ddl-why-and-how.md).


## Documentation

- [**Language Overview & Reference Manual**](docs/overview.md): Comprehensive reference covering core declarations (`sequence`, `process`, `fun`, `graph`, `extern`, `wire`), channels (`buffer`), Show-Ahead FIFOs, type system, memories, and scoping rules.

### Practical Usage Guides
- [**Guide 1: Writing Your First Pipelined Accelerator**](docs/guides/1-getting-started-pipelines.md): Dataflow stage cuts (`|||`), shift-register spanning, compiling, and visualizing.
- [**Guide 2: Control State Machines, Packet Parsers, and Register Interfaces**](docs/guides/2-fsm-and-command-processors.md): Memory-mapped CSRs, tagged union dispatchers, zero-cycle branch dispatch, and lossless relays.
- [**Guide 3: Interfacing DDL with Existing Verilog, AXI-Stream, and FPGA Pins**](docs/guides/3-interfacing-and-integration.md): Connecting to physical chip pins via `wire`, `extern` IP integration over the FIFO boundary, and AXI4-Stream master/slave wrappers.
- [**Guide 4: Memory Patterns: ROMs, Block RAMs, and Multi-Port Register Files**](docs/guides/4-memory-and-register-files.md): `lutram` vs. `bram`, zero-cost BRAM stage alignment, and multi-write register files with `--lvt-bram`.
- [**Guide 5: Simulation, Verification, and Build Workflows**](docs/guides/5-verification-and-simulation.md): Edge-triggered simulation assertions, SystemVerilog testbench templates, Verilator `-Wall` linting, and Makefiles.

### Architecture Deep Dives
- [**FIFO Boundary Adapters & Export Architecture**](docs/fifo-boundaries-and-export.md): Standard Show-Ahead (zero read latency) FIFO interfaces at module boundaries, compiler-generated adapters (`ir_adapt.rs`), use-graph root discovery, dead-code pruning, and wrapper generation (`ir_export.rs`).
- [**The Gray-Code Salt Protocol**](docs/salt-protocol.md): 2-bit Gray-code pointer mathematics, cycle-by-cycle waveform traces, skid buffer proofs, and AXI-Stream adapters.
- [**Pipeline Lowering & Memory Forwarding**](docs/pipeline-lowering.md): `sequence` stage cuts (`|||`), shift-register spanning, backpressure, and zero-cost BRAM alignment.
- [**Finite State Machine Synthesis & Scoping**](docs/fsm-synthesis.md): `process` control-flow graph construction, zero-cycle branch dispatch, lexical register allocation, and resource sharing.
- [**Multi-Write BRAM via Live Value Tables**](docs/lvt-bram-architecture.md): Synthesizing multi-write memories on FPGAs using single-write BRAM banks and distributed LVTs (`--lvt-bram`).
- [**Zero-Latency Hardware Combinators**](docs/combinators.md): Pure datapath implementation of `@merge` (rotating priority) and `@split` (lossless broadcast).
- [**Verilog Backend & Synthesizer Portability**](docs/backend-portability.md): Restrictive Verilog-2005 subset, avoiding GowinSynthesis toolchain crashes, and Verilator `-Wall` linting.
- [**GitHub CI/CD & Multi-Platform Release Pipeline**](docs/ci-release-pipeline.md): Automated multi-platform builds (Linux, Windows, macOS), nightly cron releases, and publishing runbook.

## How to use

### Get the Compiler
#### Build from sources
DDL requires a nightly Rust toolchain, pinned via `rust-toolchain.toml`:
```bash
cargo build --release
```

#### Download compiled executable

Compiled standalone binaries for Linux (x86_64, ARM64), Windows (x64), and macOS (Apple Silicon) are automatically built by GitHub Actions.

You can also download versioned releases and nightly packages directly from the **[Releases](https://github.com/balcovt/ddl/releases)** page.

Alternatively, you can download the artifact directly using the GitHub CLI:
```bash
# Download and extract the artifact for your operating system
gh run download --name x86_64-pc-windows-msvc
```

### Compile DDL to Verilog
```bash
ddl build examples/mul3.ddl -o examples/mul3.v
```

Use the `--check` flag to verify that an existing output file matches what the compiler would emit without modifying it:
```bash
ddl build examples/mul3.ddl -o examples/mul3.v --check
```

## Tooling

### Editor Support
Syntax highlighting for Visual Studio Code is available in `editors/vscode/`:
```bash
cp -r editors/vscode ~/.vscode/extensions/ddl
```

### Graph Visualization
Export Graphviz DOT graphs to visualize module connectivity and stage partitions:
```bash
ddl build examples/pipeline_graph.ddl --emit=dot | dot -Tsvg > pipeline.svg
```

## Repository Layout

```
src/
├── main.rs         CLI entry point and command-line parsing
├── lib.rs          Compiler library interface
├── driver.rs       Pipeline driver coordinating compiler passes
├── source.rs       Source file manager and import resolution
├── lex.rs          Lexer and token definitions
├── parse.rs        Recursive-descent parser and AST representation
├── symbols.rs      Symbol table and lexical scope resolution
├── ty.rs           Type inference, validation, and width checking
├── ir.rs           Typed Static Single Assignment (SSA) intermediate representation
├── ir_pipe.rs      Lowers sequence declarations into pipeline stages
├── ir_fsm.rs       Lowers process declarations into finite state machines
├── ir_graph.rs     Lowers graph declarations into structural instance netlists
├── ir_adapt.rs     Generates Show-Ahead FIFO boundary adapter modules
├── ir_export.rs    Discovers export targets, resolves use graphs, and builds wrappers
├── ir_comb.rs      Generates hardware for @merge and @split combinators
├── ir_match.rs     Lowers match expressions into case statements
├── verilog.rs      Synthesizable Verilog-2005 code generator
└── diag.rs         Source diagnostics and caret rendering
```


## License

Licensed under the [Apache License, Version 2.0](LICENSE).
