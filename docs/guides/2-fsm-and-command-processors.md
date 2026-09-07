# Guide 2: Control State Machines, Packet Parsers, and Register Interfaces

This guide demonstrates how to design control logic, command dispatchers, memory-mapped register blocks, and lossless packet relays using DDL's `process` abstraction.

---

## What You Will Learn

- How blocking operations on channels (`@rcv`, `@send`) automatically synthesize finite state machines.
- How to implement memory-mapped Control & Status Registers (CSRs).
- How to parse heterogeneous packets using tagged unions and exhaustive `match`.
- How DDL achieves zero-cycle branch dispatch on incoming commands.
- How to implement a lossless same-cycle packet relay using `@peek`, `@try_send`, and `@drop`.

---

## 1. Memory-Mapped Register Block (CSR)

In hardware systems, peripheral blocks expose Control and Status Registers (CSRs) accessible by a CPU over a bus.

Here is a 2-register peripheral implemented in DDL:

```ddl
process csr_node (cmd: buffer in u8, wdata: buffer in u32, rdata: buffer out u32)
  var reg0: u32 = 32'd0
  var reg1: u32 = 32'd0
  loop
    let c = @rcv(cmd)
    let is_write = c[0]
    let addr = c[1]
    if is_write then
      let d = @rcv(wdata)
      if addr then
        reg1 = d
      else
        reg0 = d
    else
      if addr then
        @send(rdata, reg1)
      else
        @send(rdata, reg0)
```

### How the State Machine Is Lowered:
1. **Persistent Registers**: `reg0` and `reg1` are declared at root level before the `loop`. They are synthesized as 32-bit flip-flops initialized to `0` on reset.
2. **State Partitioning**:
   - **State 0**: Waits for `cmd` to arrive (`@rcv(cmd)`).
   - **State 1 (Write branch)**: If `is_write == 1`, waits for payload data (`@rcv(wdata)`), then updates `reg0` or `reg1`.
   - **State 2 (Read branch)**: If `is_write == 0`, reads `reg0` or `reg1` and waits for `rdata` buffer capacity (`@send(rdata, ...)`).
3. **Zero-Cycle Branching**: The condition `if is_write then` is evaluated in the **exact same cycle** that `cmd` is received. There is no intermediate "decode" cycle penalty.

---

## 2. Packet Parsing with Tagged Unions

Real-world streaming protocols carry different message types with varying payloads (e.g. read requests, write requests, or heartbeats).

In SystemVerilog, this typically requires clumsy struct unions or wide buses where unused bits are padded. In DDL, you use **tagged unions**:

```ddl
struct addr_t
  page: u8
  off: u8

enum req_e
  Nop
  Read(addr_t)
  Write(u8)
  Halt

process dispatch (req: buffer in req_e, rsp: buffer out u8)
  var last_written: u8 = 8'd0
  loop
    let r = @rcv(req)
    match r
      .Nop =>
        @send(rsp, 8'd0)
      .Read a =>
        @send(rsp, a.off)
      .Write d =>
        last_written = d
        @send(rsp, last_written)
      .Halt =>
        break
```

### Safety and Synthesis Guarantees:
- **Exhaustive Matching**: The compiler verifies that every variant (`.Nop`, `.Read`, `.Write`, `.Halt`) is handled. Omitting an arm is a compile-time error.
- **Type-Safe Payloads**: `a` is typed strictly as `addr_t` (16 bits) and `d` is typed as `u8` (8 bits). You cannot access `a.off` on a `Write` packet.
- **Derived Hardware Packing**: The compiler packs the enum into `{tag, payload}` format, where the tag occupies the high bits and the payload is sized to the widest variant.
- **Loop Termination**: The `break` statement on `.Halt` cleanly halts the process.

---

## 3. High-Throughput Same-Cycle Forwarding (`@peek` + `@drop`)

When building routers, switches, or stream multiplexers, you often need to inspect an item at the head of a FIFO and conditionally forward it downstream *within the same clock cycle*.

A naive implementation that receives first can lose data if the downstream send is rejected:
```ddl
// ANTI-PATTERN (DO NOT DO THIS):
let x = @rcv(src)             // Item removed from src!
let sent = @try_send(dst, x)  // If dst is full, sent is false!
// x is now lost!
```

### The Recommended Relay Pattern:
To achieve zero data loss under backpressure, use **`@peek`**:

```ddl
process relay (src: buffer in u8, dst: buffer out u8)
  loop
    let (x, present) = @peek(src)
    if present then
      let sent = @try_send(dst, x)
      if sent then
        let took = @drop(src)
        @assert(took)
```

### How It Works:
1. **`@peek(src)`**: Reads `x` and checks if data is present **without advancing the read pointer**.
2. **`@try_send(dst, x)`**: Attempts to push `x` into `dst`. If `dst` is full, it returns `false`, leaving `src` untouched.
3. **`@drop(src)`**: Consumes the item from `src` **only after** the downstream push has succeeded.

On the clock edge:
- If `dst` had space, the write into `dst` and the drop from `src` occur simultaneously.
- If `dst` was full, the item remains at the head of `src`, ready for the next cycle.
