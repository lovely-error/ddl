# DDL: Motivation & Architectural Principles

## Why DDL?

In conventional Verilog and SystemVerilog, hardware designers spend a disproportionate amount of effort managing low-level plumbing: routing `valid`/`ready` handshakes, staging multi-cycle valid signals, building skid buffers, keeping multi-stage shift registers synchronized, resolving read-after-write memory hazards, and more. Unbuffered or naively buffered handshakes easily create combinational paths running backwards through `ready`, creating zero-delay timing loops across modular netlists, capping achievable clock frequencies ($F_{\max}$), or triggering subtle mutual-wait deadlocks that only surface late in validation or silicon bring-up.

Existing High-Level Synthesis tools attempt to address this by compiling sequential C/C++ code into hardware, but forcing a sequential software execution model onto inherently parallel, spatial silicon obscures hardware costs, produces bloated control FSMs, and frequently locks designers into proprietary vendor suites.

DDL was created to provide an abstraction built from the ground up for spatial dataflow hardware. It lets designers describe operations on data streams at a high level while preserving cycle-level predictability, clean register placement, and ensuring synthesis to standard Verilog-2005.

## How DDL Achieves Its Goals

To eliminate protocol bugs and control logic overhead while maintaining high throughput and portability, DDL is built around five core architectural pillars:

- **Isolated State & Explicit Channels**: DDL prohibits global or shared mutable state. Modules encapsulate their own registers and memories, and all inter-module communication takes place exclusively over point-to-point, backpressured channels (`buffer`).
- **Loop-Free Registered Handshakes**: Internally, channels are lowered to a 2-bit Gray-code salt protocol. Because salt pointers originate strictly from flip-flops, no combinational path ever crosses a module boundary in either direction. The 2-entry skid capacity absorbs the 1-cycle registered delay, eliminating combinational `ready` loops and mutual-wait protocol deadlocks by construction.
- **Dedicated Abstractions for Pipelines and State Machines**: DDL separates feed-forward datapaths from sequential control flow. In a pipeline (`sequence`), stage-cut operators (`|||`) delimit clock cycles while the compiler automatically synthesizes validity pipelines, backpressure gating, and multi-cycle shift registers for values spanned across stages. In a state machine (`process`), blocking operations define states while condition branches evaluate with zero-cycle decode penalties.
- **Show-Ahead FIFO Boundaries**: The internal Gray-code salt protocol remains strictly within DDL-compiled blocks. Every boundary exposed to human-written Verilog—an exported deliverable or an `extern` IP declaration—presents a standard **Show-Ahead FIFO (zero read latency)**. An item at the head of a FIFO is immediately visible on `data_read_out` whenever `has_data` is high, bridging to AMBA AXI4-Stream or Verilog testbenches with zero glue logic.
- **Predictable Storage & LVT Synthesis**: Memory primitives (`lutram` and `bram`) have deterministic latencies. Synchronous 1-cycle Block RAM reads align directly with pipeline stage cuts at zero flip-flop cost, and multi-write memories are automatically synthesized onto single-write FPGA BRAMs using distributed Live Value Tables (`--lvt-bram`).

---

## Comparison with Existing Languages & HLS Tools

To understand where DDL sits in the hardware design landscape, it helps to compare it directly against four alternative tools: **PipelineC**, **Silice**, **Xilinx C HLS (Vivado/Vitis HLS)**, and **Spade**.

### TODO