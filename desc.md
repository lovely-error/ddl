# Dataflow description language

I want something higher-level than verilog!

options
1. pipeline c
   1. c is not for hardware
   2. i dont think its good
2. silice
   1. schizo as fuck
   2. not even better than verilog!
3. xilinx stuff
   1. vendorlocked
4. google stuff
   1. meh


Two aspects:
1. computation-related constructions
   1. communicating seqv procesies 
2. io-related constructions
   1. io processies (?)



3. no global mem, every task and process can only access its local state
4. communication happen through pipes
   1. backing store mapped to ram prims of fpga
5. compute only logic in ddl, io in verilog (until io procs are finalised)

## Pieces
### basic items
1. `process`
   1. may contain `loop`s
   2. may contain state
   3. can be lowered to a state machine (never pipeline)
   4. may stop (reach terminal state)
   5. must not be nested
   6. may contain div with runtime divisor
   7. may contain operations with unpredictable latency (which?)
2. `sequence` 
   1. must not contain loops/state
      1. "inplace" mutatation can be expressed as comb logic and thus not considered state
   2. must not contain TDM memory
      1. a `bram` read spans a `|||`, where the memory's own output register
         is the pipeline register, and a `lutram` read is combinational and
         sits inside a stage
      2. every read and write of a memory a stage WRITES happens in one stage.
         Stage j and stage k hold different items in the same cycle, so a
         write in one and a read in the other pair an item's read with a write
         (k - j) items away from it, and an item's own two writes are not even
         adjacent -- the next item's lands between them. Confined to one stage
         there is no distance to depend on: every item sees every write of
         every item before it, plus its own, in source order
      3. its own it sees by FORWARDING. The array is written on the same edge
         that fills the read register, so the register holds what was there
         before the write; which addresses collided is decided in the stage
         that asked, and one bit plus the data crosses the cut
      4. a memory nothing writes is a table, has no order to keep, and may be
         read from any stage
      5. ports are as many as the source asks for, reads and writes alike: one
         write port per write that can happen in a cycle, so two writes in a
         row are two and the arms of an `if` are one. A process muxes its
         reads and its states' writes onto one port because only one state is
         current; a pipeline cannot, because every stage is live at once
      6. what that costs is the target's to decide. An ASIC memory compiler
         emits a multi-port cell; an FPGA infers no RAM at all from a second
         write port, so `--lvt-bram` builds one out of one-write blocks --
         a bank per write port, replicated per read port, and a live value
         table naming which bank holds the newest value at each address
      7. so memory latency in a pipeline is a compile-time constant of 0 or 1
         stage. `bkram`, whose point is that a bank conflict costs an extra
         cycle, can never fit that; off-chip memory belongs behind a `port`
   3. must not contain div with runtime divisor
   4. can be lowered to pipeline with prologue (head-fsm-spine-pipeline or just pipeline)
      1. head gathers inputs (first stage, blocking reads), tail computes with it (nonblocking reads are allowed)
      2. pipeline fires when all buffer sinks have slots (in presense of blocking sends)
      3. crap?
   5. `|||` stage separators are used to delimit parallel stages
   8. implicit is_valid condition at each stage
3. `function`
   1. parameters can be either
      1. direct (by value) `T`
      2. indirect (by reference)
         1. `out T` "write only"
         2. `inout T` "read write"
      3. `name(arg1:T, arg2:out T, arg3:inout T)`
   2. if contains only combinational logic (no loops) then can be used in both procs and seqvs
   3. if has dynamic loops, can only be used in procs
4. `clock`
   1. global statements for use in io procs
   2. `@wait_cycles(n)` waits n cycles in io process
   3. `@switch_to(clk_iden)` switches to a nother clock at runtime
5. `for in`
   1. `for i in 0..n`
      1. n is static integer value
   2. `for k in array_ref`
      1. iterate over items in array
6. `pin`
   1. chip IO stuff
   2. default must be specified for out and inout
   3. `pin in` `pin out` `pin inout`
   4. `@read_pin`
   5. `@write_pin`
7. `io process`
   1. io comms
8.  `struct` `union` `enum`
   1. standard datatype kinds
9.  `graph`
   1.  to specify connectivity between seqvs and processes
   2.  cycles are ok
10. `import "path.ddl"`
   1. names another file that is part of the same program
   2. no namespaces: every declaration is visible to every other one, so an
      import says which files to compile, not what to bring into scope
   3. resolved against the importing file's directory, then the `-I` path
   4. a file reached twice is included once, so diamonds and cycles are fine

### types
1. `uN` / `iN`
   1. arbitrary width unsigned (`uN`) and signed (`iN`) integers
   2. signed ints are in two complement format
   3. implicit cast to `[u1;N]`
2. `[T;n]`
   1. arrays of length n of T items
   2. `@map(item, fun_ref)` enables simd operations
   3. annotations `#[impl(...)]` to request particular impl
      1. can be either `lutram` `bram` `bkram`
      2. `lutram` ram by registers (more concurent access ports)
      3. `bram` ram by block ram (loads and stores are slower than lutram)
      4. `bkram` should synthesise as banked ram with conflict minimisation
3. pipes
   1. to connect `sequence`s and `process`ies
   2. monodirectional fifo
   3. single producer & multiple consumer (when >1 consumer, just physically duplicate sinks)
   4. blocking reads and nonblocking reads
   5. every pipe is a `buffer`
      1. producer stalls when no slots available; two slots, head and skid
      2. `@try_rcv` -> (T, u1) , if data item present, consume it (ok in seq & proc)
      3. `@rcv` -> T , blocking read (ok in proc, banned in seq)
      4. `@send` blocking send (ok in proc, banned in seq)
   6. there is no lossy kind and no way to ask for one. A producer that never
      stalled would overwrite its oldest value, and an overwrite is a dropped
      transfer -- not visible where it happens, but later and elsewhere, as a
      machine one item out of step, which is the failure mode this whole
      language exists to design out. A sink that genuinely cannot refuse ties
      `ready` high at the boundary and says so, which puts the claim where a
      reader can check it.
   7. `buffer in T` read only pipe of Ts
   8. `buffer out T` write only pipe of Ts
4. `port in T` / `port out T`
   1. the second pipe kind: a data port and an enable, and no back-pressure
      at all
   2. reached by the same operations a `buffer` is, and NOT by naming it. a
      port is not a value
      1. `@rcv` waits on the enable; `@send` never waits, because there is no
         `ready` coming back to wait for
      2. `@try_rcv` answers with the enable itself -- a port claims nothing,
         so there is no "did I take one" separate from "was there one"
      3. `@try_send` always succeeds; `@drop` is refused, having nothing to
         drop
   3. for the EDGE of the program: a pin, a PLL, a bus master that cannot be
      made to wait. the line above -- compute in ddl, io in verilog -- is what
      this is the boundary of, and a `port` is where a sink that cannot refuse
      says so
   4. between two things DDL compiled, a `buffer` is still the answer
   5. a `process` of nothing but ports is a plain verilog module, which is
      what these are for: the ordinary sequential blocks a design needs,
      written beside the dataflow ones instead of in verilog
   6. in a `sequence` a `port out` belongs to the stage that sent to it, and
      may be sent to from one stage only -- every stage is live at once

### implemented so far
The compiler in this repository accepts: `fun`, `sequence`, `process`,
`graph`, `extern`, `struct`, `enum` (with payloads), `import`, `for in`
(unrolled), `loop` nested to any depth with `break` leaving the innermost,
compound assignment, element assignment (`a[k] = v`, at a constant or computed
index), `inout` parameters, `buffer` pipes, `port in`/`port out` with an
an enable and no back-pressure, `@slice`, `{a, b}` concatenation, the `@merge`
and `@split` combinators, and `lutram` and `bram` memories — where several
reads, including reads inside an expression, each get a state of their own, and
where a `sequence` may read one across a `|||` but not write it. A blocking
operation, a `break` or a `bram` read may sit in an `if`, a `match` or a
`loop`. README.md is the current list; what follows is the design, including
the parts that are not built.

### unresolved issues
1. io procs
   1. how to use serdes io in em?
   4. how should we do cdc for io procs ?
   5. how should we clock em?
      1. every clock stmt introduces a constraint. integer multiple clocks are derived from ????

### examples
nesting by indentation instead of brackets
```
process STM (arg1: u1) -- only direct parameters (inout is forbidden)
   var state: MyEnum = MyEnum::Uninit
   var mem: [u1;32] = @zeroed()
   let some_const: u1 = 0

    loop
        match state
            Pattern1 =>
                continue
            Pattern2 =>
                break
        

-- only direct parameters, must contain parameters
sequence Exmpl (arg1: buffer in u1, arg1: buffer in u4, arg2: buffer out u4, arg3: buffer out u8)

   -- stage 1
   let val1 = @rcv(arg1) -- only first stage can contain blocking reads

   |||

   -- stage 2
   let (val2, is_valid) = @try_rcv(arg2) -- can only contain nonblocking reads

   |||

   -- stage 3
   let res: u4 = val1 * val2 -- multiplication may extend the pipeline
   @send(arg3, res) -- blocking send. the pipeline head should have checked if sink has a free slot, so this cannot be blocking


function name (arg1: u1, arg2: inout [u8;8])
   arg2[0] += arg1 -- this will be lowered differently based on whether it is used in process or sequence


pin out led_enable: u1 = 0
pin in data_pin: u1
clock ex1 = 12*10**6

io process LedBlinker (arg1: buffer out u1)

    @bind_clock(ex1)

    loop
        led_enable = !led_enable
        let smth = @read_pin(data_pin)
        @send(arg1, smth) -- blocking; the sink has a slot or the loop waits
        @wait_ticks(12*10**6)
```