// Memories inside a process that blocks.
//
// Two things had to be true before this could work, and both arrived with the
// state graph: a write has to be gated by the firing of the state that
// performs it, and a synchronous read needs a state to spend its cycle in.
//
// The second is the whole reason `bram` was refused. Its read arrives a cycle
// after its address, and until a process had states there was nowhere in the
// language to point at and say "there, that is where the cycle went".

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::verilog::EmitOptions;

fn compile(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => v,
        Err(diags) => panic!("compile failed:\n{}", map.render_all(&diags)),
    }
}

fn compile_err(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    match compile_to_verilog(&map, &EmitOptions::default()) {
        Ok(v) => panic!("expected failure, got:\n{}", v),
        Err(diags) => map.render_all(&diags),
    }
}

/// A scratchpad behind a command pipe: write on one path, read on the other.
const SCRATCH: &str = concat!(
    "process scratch (cmd: buffer in i8, din: buffer in i32, dout: buffer out i32)\n",
    "  var cells: #[impl(lutram)] [i32; 16] = @zeroed()\n",
    "  loop\n",
    "    let c = @rcv(cmd)\n",
    "    let addr: i4 = c[3..0]\n",
    "    let is_write: i1 = c[7]\n",
    "    if is_write then\n",
    "      let d = @rcv(din)\n",
    "      cells[addr] = d\n",
    "    else\n",
    "      @send(dout, cells[addr])\n",
);

#[test]
fn a_lutram_lives_in_a_blocking_process() {
    let v = compile(SCRATCH);
    assert!(v.contains("reg [31:0] cells [0:15];"), "{}", v);
    assert!(v.contains("distributed RAM"), "{}", v);
}

#[test]
fn a_write_happens_only_when_its_state_fires() {
    // Ungated, the scratchpad would be rewritten every cycle the machine sat
    // in the write state waiting for data that had not arrived.
    let v = compile(SCRATCH);
    assert!(v.contains("end else if (fire_s1) begin"), "{}", v);
    assert!(v.contains("cells[addr_r] <= din_data;"), "{}", v);
}

#[test]
fn the_address_survives_the_state_it_was_computed_in() {
    // `addr` is decoded in the command state and used a state later, so it
    // cannot be a wire.
    let v = compile(SCRATCH);
    assert!(v.contains("reg [3:0] addr_r;"), "{}", v);
    assert!(v.contains("addr_r <= (fire_s0 ? addr : addr_r);"), "{}", v);
}

#[test]
fn a_lutram_read_stays_asynchronous() {
    // Distributed RAM reads combinationally, which is the reason to choose it.
    let v = compile(SCRATCH);
    assert!(v.contains("= cells[addr_r];"), "{}", v);
}

// ---- block RAM -----------------------------------------------------------

const LOOKUP: &str = concat!(
    "process lookup (req: buffer in i8, resp: buffer out i32)\n",
    "  var table: #[impl(bram)] [i32; 256]\n",
    "  loop\n",
    "    let a = @rcv(req)\n",
    "    let v = table[a]\n",
    "    @send(resp, v)\n",
);

#[test]
fn a_bram_read_costs_a_state_and_the_state_is_visible() {
    let v = compile(LOOKUP);
    // Three states: receive the address, fetch, send. The fetch state has no
    // handshake to wait for, so it advances the cycle it is entered.
    assert!(v.contains("wire in_s1 = state == 2'd1;"), "{}", v);
    assert!(v.contains("in_s1 ? 2'd2"), "{}", v);
    assert!(!v.contains("fire_s1"), "{}", v);
}

#[test]
fn the_fetched_value_is_a_register_read_in_the_next_state() {
    let v = compile(LOOKUP);
    assert!(v.contains("reg [31:0] v_q;"), "{}", v);
    assert!(v.contains("v_q <= (in_s1 ? "), "{}", v);
    assert!(v.contains("assign resp_data = v_q;"), "{}", v);
}

#[test]
fn the_address_is_registered_before_the_fetch_state_uses_it() {
    // The fetch happens a cycle after the receive, and `req_data` belongs to
    // whatever the source is offering NOW. Reading the port directly would
    // return the value the next request asked for.
    let v = compile(LOOKUP);
    assert!(v.contains("reg [7:0] a_r;"), "{}", v);
    assert!(v.contains("= table_[a_r];"), "{}", v);
    assert!(!v.contains("table_[req_data]"), "{}", v);
}

#[test]
fn a_bram_takes_reads_and_writes_on_different_paths() {
    let v = compile(concat!(
        "process cache (cmd: buffer in i16, din: buffer in i32, dout: buffer out i32)\n",
        "  var table: #[impl(bram)] [i32; 256]\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    let addr: i8 = c[7..0]\n",
        "    let is_write: i1 = c[15]\n",
        "    if is_write then\n",
        "      let d = @rcv(din)\n",
        "      table[addr] = d\n",
        "    else\n",
        "      let v = table[addr]\n",
        "      @send(dout, v)\n",
    ));
    assert!(v.contains("block RAM"), "{}", v);
    assert!(v.contains("table_[addr_r] <= din_data;"), "{}", v);
    assert!(v.contains("v_q <= (in_s2 ? "), "{}", v);
}

#[test]
fn a_bram_read_inside_an_expression_says_how_to_bind_it() {
    // There is no way to say that the rest of the expression waits a cycle,
    // so the read has to be its own statement.
    let text = compile_err(concat!(
        "process p (req: buffer in i8, resp: buffer out i32)\n",
        "  var table: #[impl(bram)] [i32; 256]\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    @send(resp, table[a])\n",
    ));
    assert!(text.contains("cannot sit inside an expression"), "{}", text);
    assert!(text.contains("let x = table[i]"), "{}", text);
}

#[test]
fn a_bram_needs_a_process_with_states() {
    let text = compile_err(concat!(
        "process p (addr: buffer in i5, rd: buffer out i32)\n",
        "  var vals: #[impl(bram)] [i32; 32]\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("no state to put it in"), "{}", text);
}

#[test]
fn a_bram_with_a_reset_is_refused_with_the_cost_named() {
    let text = compile_err(concat!(
        "process p (req: buffer in i8, resp: buffer out i32)\n",
        "  var table: #[impl(bram)] [i32; 256] = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    let v = table[a]\n",
        "    @send(resp, v)\n",
    ));
    assert!(text.contains("cannot be reset"), "{}", text);
}

#[test]
fn bkram_is_still_refused_and_says_why() {
    let text = compile_err(concat!(
        "process p (req: buffer in i8, resp: buffer out i32)\n",
        "  var table: #[impl(bkram)] [i32; 256]\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    let v = table[a]\n",
        "    @send(resp, v)\n",
    ));
    assert!(text.contains("`#[impl(bkram)]` is not supported yet"), "{}", text);
    assert!(text.contains("conflict model"), "{}", text);
}
