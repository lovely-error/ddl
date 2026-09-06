# Remaining compiler defects found during the probe repair review

Verified 2026-09-06 against the compiler after the probe repairs and nested
mutable lifetime implementation. These are additional unresolved defects;
they are not covered by the passing probe regression result.

## Scalar selection and sign extension

`review-reproducers/edge_cases.ddl` contains three accepted functions using
`@sext(x, 8)`, `@trunc(x, 1)`, and `x[0]` with `x: u1`. The backend declares
`x` as a scalar but emits `x[0]`. Questa 2025.2 rejects each with vlog-10008:
"Too many indices (1) for array type x (dimensionality 0)."

Fix selection rendering to account for scalar operands. Identity selections
and truncations should return the operand, and scalar sign extension should
replicate the operand without indexing it. Keep named operands where a real
vector selection requires them.

## Public identifier collision

The same DDL file declares inputs named `cell` and `cell_`. Reserved-word
sanitization turns both into `cell_`, producing duplicate ports and merging
their references. Questa rejects the module with vlog-13311 and vlog-2388.

Use a collision-free interface name mapping consistently for declarations
and instance connections, or diagnose the collision. The existing internal
register and memory naming fix does not cover public interface collisions.

To reproduce both backend failures in PowerShell:

```powershell
cargo build --locked
New-Item -ItemType Directory -Force target/review | Out-Null
./target/debug/ddl.exe build docs/review-reproducers/edge_cases.ddl -o target/review/edge_cases.v
# With the Questa executables on PATH:
vlib target/review/work
vlog -work target/review/work target/review/edge_cases.v
```

DDL succeeds; the last command is expected to fail until these defects are
fixed. These are review inputs, not tests that currently pass.

## Rust AST lifetime API

`review-reproducers/lifetime_api.rs` demonstrates two safe-caller API holes:

- A caller can move `Parsed.decls` out of its lifetime-bearing wrapper and
  return an AST after destroying its source map.
- `anumspan_to_str` returns a caller-chosen lifetime unrelated to the source
  buffer, allowing a string reference to outlive its allocation.

The examples were verified by compilation only. No dangling memory reads
were executed. This establishes a Rust library API soundness problem, not
an observed malformed-DDL exploit of the command-line compiler.

```powershell
cargo build --locked
New-Item -ItemType Directory -Force target/review | Out-Null
rustc --edition=2024 --crate-type=lib --emit=metadata --extern ddl=target/debug/libddl.rlib -L dependency=target/debug/deps docs/review-reproducers/lifetime_api.rs -o target/review/lifetime_api.rmeta
```

The command currently succeeds. A repaired ownership API must prevent these
escapes. Prefer source identifiers and offsets resolved through a borrowed
source map, or fully lifetime-bearing AST nodes with private constructors.
`PhantomData` on a wrapper alone cannot protect extractable raw-pointer fields.
