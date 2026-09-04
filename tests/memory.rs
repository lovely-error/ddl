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
    "process scratch (cmd: buffer in u8, din: buffer in u32, dout: buffer out u32)\n",
    "  var cells: #[impl(lutram)] [u32; 16] = @zeroed()\n",
    "  loop\n",
    "    let c = @rcv(cmd)\n",
    "    let addr: u4 = c[3..0]\n",
    "    let is_write: u1 = c[7]\n",
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
    assert!(v.contains("cells[addr_r] <= din_item;"), "{}", v);
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
    "process lookup (req: buffer in u8, resp: buffer out u32)\n",
    "  var table: #[impl(bram)] [u32; 256]\n",
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
fn the_read_happens_inside_the_memorys_own_clocked_block() {
    // This is the whole difference between `bram` and `lutram`, and it is the
    // difference a synthesizer looks for. A `wire q = mem[addr];` with the
    // flop in some other always block is a combinational array read plus a
    // register, and infers what `lutram` already gives you plus the flop.
    let v = compile(LOOKUP);
    assert!(v.contains("reg [31:0] table__q;"), "{}", v);
    assert!(v.contains("if (in_s1) table__q <= table_[a_r];"), "{}", v);
    // The array is never read outside that block. Checked against the ARRAY
    // rather than against "no wire of that width": the read register now feeds
    // an entry, so a wire carrying `table__q` is expected and says nothing
    // about where the array was touched.
    for line in v.lines().filter(|l| l.trim_start().starts_with("wire ")) {
        assert!(!line.contains("table_["), "the array is read combinationally:
{}", line);
    }
    // The read register feeds an ENTRY, which is then published as the pair.
    // A `bram` read still costs the state it always did; what changed is where
    // its answer lands.
    assert!(v.contains("reg [31:0] table__q;"), "{}", v);
    assert!(v.contains("assign resp_data = {resp_e1, resp_e0};"), "{}", v);
}

#[test]
fn the_read_register_belongs_to_the_memory_not_to_the_state() {
    // One output register per port, however many states read it, because that
    // is what the hardware has.
    let v = compile(LOOKUP);
    assert_eq!(v.matches("table__q <=").count(), 1, "{}", v);
    // And it is declared with the array, not with the state machine's
    // registers.
    let decl = v.find("reg [31:0] table_ [0:255];").expect("the array");
    let q = v.find("reg [31:0] table__q;").expect("the read register");
    assert!(q > decl, "the read register should follow the array it belongs to:
{}", v);
}

#[test]
fn the_address_is_registered_before_the_fetch_state_uses_it() {
    // The fetch happens a cycle after the receive, and `req_data` belongs to
    // whatever the source is offering NOW. Reading the port directly would
    // return the value the next request asked for.
    let v = compile(LOOKUP);
    assert!(v.contains("reg [7:0] a_r;"), "{}", v);
    assert!(v.contains("= table_[a_r];"), "{}", v);
    assert!(!v.contains("table_[req_item]"), "{}", v);
}

#[test]
fn a_bram_takes_reads_and_writes_on_different_paths() {
    let v = compile(concat!(
        "process cache (cmd: buffer in u16, din: buffer in u32, dout: buffer out u32)\n",
        "  var table: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    let addr: u8 = c[7..0]\n",
        "    let is_write: u1 = c[15]\n",
        "    if is_write then\n",
        "      let d = @rcv(din)\n",
        "      table[addr] = d\n",
        "    else\n",
        "      let v = table[addr]\n",
        "      @send(dout, v)\n",
    ));
    assert!(v.contains("block RAM"), "{}", v);
    // Both ports in one clocked block, which is the simple-dual-port template.
    assert!(v.contains("table_[addr_r] <= din_item;"), "{}", v);
    assert!(v.contains("table__q <= table_["), "{}", v);
    let block = v.split("// table_ [0:255]").nth(1).expect("the memory block");
    let block = block.split("endmodule").next().expect("the end");
    assert_eq!(block.matches("always @(posedge clk)").count(), 1, "{}", block);
}

#[test]
fn a_bram_read_inside_an_expression_gets_a_state_of_its_own() {
    // This used to be a diagnostic: "a read of `table` takes a cycle, so it
    // cannot sit inside an expression". The cycle is real, but a state to
    // spend it in is something the compiler can supply -- the read is lifted
    // onto a line of its own and the expression reads the name.
    let v = compile(concat!(
        "process p (req: buffer in u8, resp: buffer out u32)\n",
        "  var table: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    @send(resp, table[a])\n",
    ));
    // Receive, fetch, send -- the fetch is the state the cycle went into.
    assert!(v.contains("in_s2"), "{}", v);
    assert!(!v.contains("in_s3"), "{}", v);
    assert!(v.contains("if (in_s1) table__q <= table_[a_r];"), "{}", v);
}

#[test]
fn a_lifted_read_is_still_refused_where_there_is_no_state() {
    // Lifting needs somewhere to lift TO. A process with no blocking operation
    // has one pass per cycle and no states, so the cycle has nowhere to go and
    // the answer is still no -- with the diagnostic that says which.
    let text = compile_err(concat!(
        "process p (req: buffer in u8, resp: buffer out u32)\n",
        "  var table: #[impl(bram)] [u32; 256]\n",
        "  let (a, got) = @try_rcv(req)\n",
        "  let _s = @try_send(resp, table[a] + 32'd1)\n",
    ));
    assert!(text.contains("no state to put it in"), "{}", text);
}

#[test]
fn a_bram_needs_a_process_with_states() {
    let text = compile_err(concat!(
        "process p (addr: buffer in u5, rd: buffer out u32)\n",
        "  var vals: #[impl(bram)] [u32; 32]\n",
        "  let (a, got) = @try_rcv(addr)\n",
        "  let _s = @try_send(rd, vals[a])\n",
    ));
    assert!(text.contains("no state to put it in"), "{}", text);
}

#[test]
fn a_bram_with_a_reset_is_refused_with_the_cost_named() {
    let text = compile_err(concat!(
        "process p (req: buffer in u8, resp: buffer out u32)\n",
        "  var table: #[impl(bram)] [u32; 256] = @zeroed()\n",
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
        "process p (req: buffer in u8, resp: buffer out u32)\n",
        "  var table: #[impl(bkram)] [u32; 256]\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    let v = table[a]\n",
        "    @send(resp, v)\n",
    ));
    assert!(text.contains("`#[impl(bkram)]` is not supported yet"), "{}", text);
    assert!(text.contains("conflict model"), "{}", text);
}

// ---- more than one synchronous read ---------------------------------------
//
// One read hid three bugs, because with one read nothing overwrites the
// memory's output register and a name holding it stays accidentally correct.
//
//   * the read's binding was not counted as something the state DEFINES, so
//     it never got a register and `x + y` came out as `y + y`;
//   * the read port's address and enable were not roots of the live-value
//     walk, so with two reads the memory's clocked block referred to wires the
//     file never declared;
//   * statements after the read were absorbed into the state that PRESENTS the
//     address, where the value has not arrived yet.

/// Two reads of one `bram`, added together.
const TWO_READS: &str = concat!(
    "process two (a: buffer in u8, o: buffer out u32)\n",
    "  var m: #[impl(bram)] [u32; 256]\n",
    "  loop\n",
    "    let i = @rcv(a)\n",
    "    let x = m[i]\n",
    "    let y = m[8'd7]\n",
    "    @send(o, x + y)\n",
);

#[test]
fn two_reads_do_not_collapse_onto_one_register() {
    let v = compile(TWO_READS);
    // The sum is of two saved values, not of the output register twice.
    assert!(!v.contains("m_q + m_q"), "the two reads collapsed:\n{}", v);
    assert!(v.contains("x_r"), "{}", v);
    assert!(v.contains("y_r"), "{}", v);
}

#[test]
fn a_saved_read_is_captured_the_cycle_after_its_state() {
    let v = compile(TWO_READS);
    // The array is read at the clock edge that ENDS the state presenting the
    // address, so a copy taken on that state's firing takes the previous read.
    assert!(v.contains("rd_valid_s1 <= in_s1"), "{}", v);
    assert!(v.contains("x_r <= (rd_valid_s1 ?"), "{}", v);
    // And for the one cycle before the copy lands, the name is the register
    // itself -- otherwise the state right after the fetch could not use it.
    assert!(v.contains("x_live = rd_valid_s1 ?"), "{}", v);
}

#[test]
fn one_read_costs_no_register_of_its_own() {
    // The saving is only needed where another read can overwrite the port.
    // With one read the output register IS the answer, and a flop plus a mux
    // per read would be real area in every process that reads a `bram` once.
    let v = compile(concat!(
        "process one (a: buffer in u8, o: buffer out u32)\n",
        "  var m: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let i = @rcv(a)\n",
        "    let x = m[i]\n",
        "    @send(o, x)\n",
    ));
    assert!(!v.contains("rd_valid"), "{}", v);
    assert!(!v.contains("x_r"), "{}", v);
}

#[test]
fn the_read_port_is_declared_when_two_states_share_it() {
    let v = compile(TWO_READS);
    // Everything the memory's clocked block names has to be declared. The
    // enable is an OR and the address a mux, and nothing else reaches either.
    for line in v.lines() {
        let Some(rest) = line.trim().strip_prefix("if (") else {
            continue;
        };
        let Some(cond) = rest.split(')').next() else {
            continue;
        };
        let is_temp = cond.starts_with('n') && cond[1..].chars().all(|c| c.is_ascii_digit());
        if is_temp {
            assert!(
                v.contains(&format!("wire {} =", cond)),
                "{} is used and never declared:\n{}",
                cond,
                v
            );
        }
    }
}

// ---- a read inside an expression ------------------------------------------
//
// "a read of `m` takes a cycle, so it cannot sit inside an expression" was a
// scheduling limit dressed as a language rule. The reads in an expression are
// ordinary reads in a fixed order, so each gets a state and the expression
// reads the names.

#[test]
fn reads_in_an_expression_each_get_a_state() {
    let v = compile(concat!(
        "process expr (a: buffer in u8, o: buffer out u32)\n",
        "  var m: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let i = @rcv(a)\n",
        "    let s = m[i] + m[8'd7]\n",
        "    @send(o, s)\n",
    ));
    // Four states: receive, fetch, fetch, send.
    assert!(v.contains("in_s3"), "{}", v);
    assert!(!v.contains("in_s4"), "{}", v);
    assert!(v.contains("rd_valid_s1"), "{}", v);
    assert!(v.contains("rd_valid_s2"), "{}", v);
}

#[test]
fn a_read_in_a_sent_value_is_lifted() {
    let v = compile(concat!(
        "process sendread (a: buffer in u8, o: buffer out u32)\n",
        "  var m: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let i = @rcv(a)\n",
        "    @send(o, m[i])\n",
    ));
    assert!(v.contains("m_q"), "{}", v);
    // Receive, fetch, send.
    assert!(v.contains("in_s2"), "{}", v);
}

#[test]
fn a_read_inside_a_branch_stays_in_that_branch() {
    // The cycle a read costs is spent only on the path that reads, so lifting
    // one out of an arm would make the other arm pay for it.
    let v = compile(concat!(
        "process armed (a: buffer in u8, o: buffer out u32)\n",
        "  var m: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let i = @rcv(a)\n",
        "    if i[7] then\n",
        "      @send(o, m[i] + 32'd1)\n",
        "    else\n",
        "      @send(o, 32'd0)\n",
    ));
    assert!(v.contains("m_q"), "{}", v);
    assert!(v.contains("branch_s0"), "{}", v);
}

#[test]
fn a_write_target_is_not_lifted_as_a_read() {
    // `m[a] = d` is the write port. Lifting the subscript would turn every
    // store into a load and then assign to what it loaded.
    let v = compile(concat!(
        "process store (a: buffer in u8, d: buffer in u32, o: buffer out u32)\n",
        "  var m: #[impl(bram)] [u32; 256]\n",
        "  loop\n",
        "    let i = @rcv(a)\n",
        "    let x = @rcv(d)\n",
        "    m[i] = x\n",
        "    @send(o, 32'd0)\n",
    ));
    assert!(v.contains("m[i_r] <="), "{}", v);
}

// ---- what the annotation says when it is wrong ---------------------------
//
// Past the `#[` there is no other construct the text could be, so every
// failure below is a real mistake rather than "not this one". The parser
// answers `Result<T, ()>`, so it leaves the message behind for `parse_source`
// to pick up rather than returning it.

#[test]
fn an_annotation_that_is_not_impl_names_itself() {
    let text = compile_err(concat!(
        "process p (rd: buffer out u32)\n",
        "  var v: #[inline(lutram)] [u32; 4] = @zeroed()\n",
        "  let _s = @try_send(rd, v[2'd0])\n",
    ));
    assert!(text.contains("`inline` is not an annotation"), "{}", text);
    assert!(text.contains("t.ddl:2:12"), "{}", text);
}

#[test]
fn an_annotation_on_a_single_value_says_it_wanted_an_array() {
    // `#[impl(bram)] u32` asks for a memory holding one thing, which is a
    // register with extra words.
    let text = compile_err(concat!(
        "process p (rd: buffer out u32)\n",
        "  var v: #[impl(lutram)] u32 = @zeroed()\n",
        "  let _s = @try_send(rd, v)\n",
    ));
    assert!(text.contains("needs an array"), "{}", text);
    assert!(text.contains("a single value is a register"), "{}", text);
}

#[test]
fn an_unclosed_annotation_points_at_where_the_bracket_belongs() {
    let text = compile_err(concat!(
        "process p (rd: buffer out u32)\n",
        "  var v: #[impl(lutram] [u32; 4] = @zeroed()\n",
        "  let _s = @try_send(rd, v[2'd0])\n",
    ));
    assert!(text.contains("closes with `)]`"), "{}", text);
}
