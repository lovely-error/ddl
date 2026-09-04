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

/// Compiles with `--lvt-bram`, which changes only how a multi-write `bram` is
/// built out of primitives.
fn compile_lvt(src: &str) -> String {
    let map = SourceMap::new("t.ddl", src);
    let opts = EmitOptions { lvt_bram: true, ..EmitOptions::default() };
    match compile_to_verilog(&map, &opts) {
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

// ---- memories in a sequence -----------------------------------------------
//
// A pipeline stage cut IS a clock edge, which is the one thing a `bram` read
// needs and could not find here before: the address goes out in the stage that
// names it and the value is there in the stage below, in the memory's own
// output register. That register is the pipeline register for that boundary,
// so the read costs no flop of ours and no cycle beyond the cut that was
// already written.
//
// A stage MAY write one, under a rule: every read and write of a memory that
// is written happens in ONE stage. Every stage is live at once holding a
// different item, so a write in stage j and a read in stage k pair an item's
// read with a write belonging to an item (k - j) places away in the stream --
// and an item's own two writes would not even be adjacent, the next item's
// landing between them. With one stage there is no distance to depend on, and
// the guarantee is the one the source reads as: each item sees every write of
// every item before it, plus its own, in source order.
//
// A memory NOTHING writes is a table, has no order to keep, and may be read
// from any stage, any number of times.

/// The shape the whole feature exists for: a lookup, one item per cycle.
const SEQ_LOOKUP: &str = concat!(
    "sequence lookup (addr: buffer in u5, val: buffer out u32)\n",
    "  var tbl: #[impl(bram)] [u32; 32]\n",
    "  let a = @rcv(addr)\n",
    "  let v = tbl[a]\n",
    "  |||\n",
    "  @send(val, v)\n",
);

#[test]
fn a_bram_read_spans_a_stage_cut() {
    let v = compile(SEQ_LOOKUP);
    // The array and its output register, in the memory's own clocked block --
    // the shape a synthesizer infers block RAM from.
    assert!(v.contains("reg [31:0] tbl [0:31];"), "{}", v);
    assert!(v.contains("reg [31:0] tbl_q;"), "{}", v);
    assert!(v.contains("if (tbl_re) tbl_q <= tbl["), "{}", v);
}

#[test]
fn the_memorys_register_is_the_pipeline_register() {
    // No `v_s1`. The name the read bound crosses the cut in `tbl_q`, and a
    // register of our own beside it would pair the value with the item behind.
    let v = compile(SEQ_LOOKUP);
    assert!(!v.contains("v_s1"), "a second register for the read:\n{}", v);
}

#[test]
fn the_read_enable_follows_the_shift() {
    // A memory that kept reading through a stall would have moved on to the
    // item behind by the time the stall lifted.
    let v = compile(SEQ_LOOKUP);
    assert!(v.contains("wire tbl_re = "), "{}", v);
    assert!(v.contains("& shift;"), "{}", v);
}

#[test]
fn a_sequence_memory_is_a_rom_in_the_comment_too() {
    let v = compile(SEQ_LOOKUP);
    assert!(v.contains("block ROM: sync reads"), "{}", v);
    assert!(!v.contains("one sync write port"), "{}", v);
}

#[test]
fn the_value_is_not_there_in_the_stage_that_asked() {
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  let v = tbl[a]\n",
        "  let w: u32 = v + 32'd1\n",
        "  |||\n",
        "  @send(val, w)\n",
    ));
    assert!(text.contains("used in the stage that asked for it"), "{}", text);
}

#[test]
fn a_read_in_the_last_stage_has_nowhere_to_land() {
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  |||\n",
        "  let v = tbl[a]\n",
        "  @send(val, v)\n",
    ));
    assert!(text.contains("read in the last stage"), "{}", text);
    assert!(text.contains("put a `|||` after this line"), "{}", text);
}

#[test]
fn a_table_may_be_read_from_several_stages() {
    // Nothing writes it, so there is no order to keep and no reason to confine
    // it to one stage. Each read takes its own port and its own enable, from
    // the liveness of the stage that asked.
    let v = compile(concat!(
        "sequence s (req: buffer in u8, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 16]\n",
        "  let c = @rcv(req)\n",
        "  let x = tbl[c[3..0]]\n",
        "  |||\n",
        "  let y = tbl[c[7..4]]\n",
        "  |||\n",
        "  @send(val, x + y)\n",
    ));
    assert!(v.contains("wire tbl_re1 = v0 & shift;"), "{}", v);
    assert!(v.contains("x_s2 <= (shift ? tbl_q0 : x_s2);"), "{}", v);
}

#[test]
fn a_table_takes_a_read_port_per_read_in_one_stage() {
    // Two reads in one stage are two addresses in the same cycle and there is
    // nothing to mux them onto, so they are two ports -- which an ASIC memory
    // compiler answers with a two-read cell and an FPGA by replicating the
    // array under the same write stream. Unlike a process, where one state is
    // current and several reads DO mux onto one port.
    let v = compile(concat!(
        "sequence s (req: buffer in u8, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 16]\n",
        "  let c = @rcv(req)\n",
        "  let x = tbl[c[3..0]]\n",
        "  let y = tbl[c[7..4]]\n",
        "  |||\n",
        "  @send(val, x + y)\n",
    ));
    assert!(v.contains("reg [31:0] tbl_q0;"), "{}", v);
    assert!(v.contains("reg [31:0] tbl_q1;"), "{}", v);
    assert!(v.contains("if (tbl_re0) tbl_q0 <= tbl["), "{}", v);
    assert!(v.contains("if (tbl_re1) tbl_q1 <= tbl["), "{}", v);
}


#[test]
fn a_written_memory_is_owned_by_one_stage() {
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  tbl[a] = 32'd7\n",
        "  let v = tbl[a]\n",
        "  |||\n",
        "  let w = tbl[a]\n",
        "  |||\n",
        "  @send(val, v + w)\n",
    ));
    assert!(text.contains("touched in stage 0 and stage 1"), "{}", text);
    assert!(text.contains("live at once"), "{}", text);
}

#[test]
fn a_write_is_gated_by_the_stage_that_owns_it_and_no_other() {
    // Ungated, a stall would rewrite every cycle it waited. Gated by every
    // stage rather than by the owning one, the write would need the whole
    // pipeline occupied before it happened -- which is what this pins.
    let v = compile(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  tbl[a] = 32'd7\n",
        "  let x: u32 = @zext(a, 32)\n",
        "  |||\n",
        "  let y: u32 = x + 32'd1\n",
        "  |||\n",
        "  @send(val, y)\n",
    ));
    let block = v.split("// tbl [0:31]").nth(1).expect("the memory block");
    let block = block.split("endmodule").next().expect("the end");
    assert!(block.contains("& shift)"), "{}", block);
    assert!(!block.contains("v0 & shift"), "gated by a stage that does not own it:\n{}", block);
    assert!(!block.contains("v1 & shift"), "gated by a stage that does not own it:\n{}", block);
}

#[test]
fn a_read_sees_a_write_above_it_in_the_same_stage() {
    // Different addresses, so the collision is a real comparison. It is decided
    // in the stage that asked -- where both addresses exist -- and one bit plus
    // the data crosses the cut to answer the read on the far side.
    let v = compile(concat!(
        "sequence s (req: buffer in u8, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 16]\n",
        "  let c = @rcv(req)\n",
        "  let wa: u4 = c[3..0]\n",
        "  let ra: u4 = c[7..4]\n",
        "  tbl[wa] = 32'd7\n",
        "  let v = tbl[ra]\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(v.contains("reg tbl_fwd0_s1;"), "{}", v);
    assert!(v.contains("tbl_fwd0_s1 <= (shift ? (ra == wa) : tbl_fwd0_s1);"), "{}", v);
    assert!(v.contains("tbl_fwd0_s1 ? tbl_wdata0_s1 : tbl_q"), "{}", v);
}

#[test]
fn an_unconditional_write_to_the_address_read_needs_no_port_at_all() {
    // `t[a] = v` then `t[a]` is `v`. Asking the array as well would be a read
    // port, an output register and a mux that can only choose one way.
    let v = compile(concat!(
        "sequence s (req: buffer in u4, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 16]\n",
        "  let a = @rcv(req)\n",
        "  tbl[a] = 32'd7\n",
        "  let v = tbl[a]\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(!v.contains("tbl_q"), "a read port for a value already in hand:\n{}", v);
    assert!(!v.contains("tbl_re"), "{}", v);
    // The written value crosses the cut like any other stage-0 value.
    assert!(v.contains("v_s1 <= (shift ? 32'd7 : v_s1);"), "{}", v);
    // And the memory is still written, for the items behind this one.
    assert!(v.contains("tbl[req_item] <= 32'd7;"), "{}", v);
}

#[test]
fn a_sequence_bram_cannot_be_reset_either() {
    // The process's rule, unchanged and for the same measured reason: a reset
    // is a write to every element and infers flip-flops, not a block RAM.
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32] = @zeroed()\n",
        "  let a = @rcv(addr)\n",
        "  let v = tbl[a]\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(text.contains("cannot be reset"), "{}", text);
}

#[test]
fn a_lutram_read_in_a_sequence_costs_no_cut() {
    // Asynchronous, so it is ordinary combinational work inside one stage --
    // and several of them are several read ports, which is what distributed
    // RAM is for.
    let v = compile(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(lutram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  let v: u32 = tbl[a] + tbl[a]\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(!v.contains("tbl_q"), "a lutram has no read register:\n{}", v);
    // Two reads, two array reads, in the one stage that wrote them. A
    // synchronous read could not have done that at any price.
    assert_eq!(v.matches("tbl[addr_item]").count(), 2, "{}", v);
    // And nothing clocked at all: no write port, and reads that are wires.
    assert!(!v.contains("// tbl "), "{}", v);
}

#[test]
fn a_read_two_stages_on_crosses_like_anything_else() {
    // `tbl_q` holds this item's answer for exactly one shift, so a use two
    // stages down needs the ordinary crossing register -- captured from
    // `tbl_q` at the SECOND cut, not the first.
    let v = compile(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  let v = tbl[a]\n",
        "  |||\n",
        "  let w: u32 = @zext(a, 32)\n",
        "  |||\n",
        "  @send(val, v + w)\n",
    ));
    assert!(v.contains("v_s2 <= (shift ? tbl_q : v_s2);"), "{}", v);
    assert!(!v.contains("v_s1"), "registered at the first cut too:\n{}", v);
}

#[test]
fn a_bram_read_in_a_sequence_expression_says_where_the_cycle_goes() {
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  let v: u32 = tbl[a] + 32'd1\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(text.contains("cannot sit inside an expression"), "{}", text);
    // The note names a stage cut here, not a state.
    assert!(text.contains("put a `|||` after it"), "{}", text);
}

#[test]
fn bkram_is_refused_in_a_sequence_too() {
    let text = compile_err(concat!(
        "sequence s (addr: buffer in u5, val: buffer out u32)\n",
        "  var tbl: #[impl(bkram)] [u32; 32]\n",
        "  let a = @rcv(addr)\n",
        "  let v = tbl[a]\n",
        "  |||\n",
        "  @send(val, v)\n",
    ));
    assert!(text.contains("`#[impl(bkram)]` is not supported yet"), "{}", text);
}

// ---- write-then-read in one state, which was silently wrong ---------------

#[test]
fn a_lutram_read_sees_a_write_above_it_in_the_same_state() {
    // The array updates on the clock edge, so `cells[a] = d` followed by
    // `cells[a]` read the contents from BEFORE the write -- with no diagnostic,
    // for as long as memories have existed. `bram` escaped it by accident: its
    // read is scheduled into a state after any write's, so the array had
    // already been updated by the time it looked.
    let v = compile(concat!(
        "process p (req: buffer in u4, dout: buffer out u32)
",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()
",
        "  loop
",
        "    let a = @rcv(req)
",
        "    cells[a] = 32'd7
",
        "    let v = cells[a]
",
        "    @send(dout, v)
",
    ));
    // Unconditional and the same address, so the read IS the written value and
    // the array read folds away entirely.
    assert!(!v.contains("= cells[req_item];"), "still reads the stale array:
{}", v);
    assert!(v.contains("cells[req_item] <= 32'd7;"), "{}", v);
}

#[test]
fn a_guarded_write_forwards_under_its_own_guard() {
    let v = compile(concat!(
        "process p (cmd: buffer in u16, dout: buffer out u32)
",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()
",
        "  loop
",
        "    let c = @rcv(cmd)
",
        "    let a: u4 = c[3..0]
",
        "    let b: u4 = c[7..4]
",
        "    if c[15] then
",
        "      cells[a] = 32'd7
",
        "    let v = cells[b]
",
        "    @send(dout, v)
",
    ));
    // The read takes the pending write only where the guard held AND the
    // addresses matched; otherwise the array, as before.
    assert!(v.contains("(b == "), "no address comparison:
{}", v);
    assert!(v.contains("cells[b]"), "{}", v);
}

#[test]
fn a_read_with_no_write_above_it_is_untouched() {
    // The common case must emit what it emitted before forwarding existed: no
    // comparator, no mux, just the array read.
    let v = compile(concat!(
        "process p (req: buffer in u4, dout: buffer out u32)
",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()
",
        "  loop
",
        "    let a = @rcv(req)
",
        "    let v = cells[a]
",
        "    @send(dout, v)
",
    ));
    assert!(v.contains("wire [31:0] v = cells[req_item];"), "{}", v);
}

// ---- write ports are plural -----------------------------------------------
//
// How many a memory has is how many writes ONE PATH performs, not how many the
// source contains. Two writes in a row both happen this cycle and there is
// nothing to mux them onto; the two arms of an `if` cannot both happen, and the
// SSA join is what proves it -- both arms write slot 0, and the join brings
// them back together as one port with a muxed address.

#[test]
fn two_writes_in_a_row_are_two_ports() {
    // Before this the second replaced the first in the environment and the
    // first was silently dropped: the source said write twice and the hardware
    // wrote once, with nothing said about it.
    let v = compile(concat!(
        "process p (req: buffer in u4, dout: buffer out u32)\n",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    cells[a] = 32'd7\n",
        "    cells[a + 4'd1] = 32'd9\n",
        "    @send(dout, 32'd0)\n",
    ));
    assert!(v.contains("cells[req_item] <= 32'd7;"), "{}", v);
    assert!(v.contains("cells[(req_item + 4'd1)] <= 32'd9;"), "{}", v);
    assert!(v.contains("2 sync write ports"), "{}", v);
}

#[test]
fn the_two_arms_of_an_if_share_one_port() {
    // They cannot both happen, so one port with a muxed address and data is
    // the whole of what the hardware needs -- and on an FPGA a second port is
    // the difference between a RAM primitive and a pile of flip-flops.
    let v = compile(concat!(
        "process p (cmd: buffer in u16, dout: buffer out u32)\n",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    let a: u4 = c[3..0]\n",
        "    let b: u4 = c[7..4]\n",
        "    if c[15] then\n",
        "      cells[a] = 32'd2\n",
        "    else\n",
        "      cells[b] = 32'd3\n",
        "    @send(dout, 32'd0)\n",
    ));
    assert!(v.contains("one sync write port"), "{}", v);
    // And the enable is not `c ? 1'b1 : 1'b1`: both arms write unconditionally,
    // so the shared port's enable is whatever reaching the `if` costs.
    assert!(!v.contains("1'b1 : 1'b1"), "{}", v);
}

#[test]
fn an_if_with_no_else_is_still_one_port_with_the_guard_as_its_enable() {
    let v = compile(concat!(
        "process p (cmd: buffer in u16, dout: buffer out u32)\n",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    if c[15] then\n",
        "      cells[c[3..0]] = 32'd2\n",
        "    @send(dout, 32'd0)\n",
    ));
    assert!(v.contains("one sync write port"), "{}", v);
    assert!(v.contains("cmd_item[15]"), "{}", v);
}

#[test]
fn match_arms_share_one_port_too() {
    let v = compile(concat!(
        "enum op_e: u2\n",
        "  A\n",
        "  B\n",
        "  C\n",
        "process p (cmd: buffer in op_e, dout: buffer out u32)\n",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()\n",
        "  loop\n",
        "    let c = @rcv(cmd)\n",
        "    match c\n",
        "      .A =>\n",
        "        cells[4'd0] = 32'd1\n",
        "      .B =>\n",
        "        cells[4'd1] = 32'd2\n",
        "      _ =>\n",
        "        cells[4'd2] = 32'd3\n",
        "    @send(dout, 32'd0)\n",
    ));
    assert!(v.contains("one sync write port"), "{}", v);
}

#[test]
fn several_states_writing_one_port_is_still_one_port() {
    // Only one state is current in any cycle, so they mux onto it. This is the
    // rule that did not change, and the one a pipeline cannot use.
    let v = compile(SCRATCH);
    assert!(v.contains("one sync write port"), "{}", v);
}

#[test]
fn two_writes_in_a_stage_are_two_ports_and_a_read_sees_both() {
    let v = compile(concat!(
        "sequence s (req: buffer in u8, dout: buffer out u32)\n",
        "  var tbl: #[impl(lutram)] [u32;16]\n",
        "  let c = @rcv(req)\n",
        "  tbl[c[3..0]] = 32'd1\n",
        "  tbl[c[7..4]] = 32'd2\n",
        "  let v = tbl[c[3..0]]\n",
        "  |||\n",
        "  @send(dout, v)\n",
    ));
    assert!(v.contains("2 sync write ports"), "{}", v);
    // Source order: the later write wins the forwarding mux, as it wins the
    // array, so what the read saw and what the array holds cannot disagree.
    assert!(v.contains("32'd2 : 32'd1"), "{}", v);
}

// ---- --lvt-bram ------------------------------------------------------------

const TWO_WRITE_BRAM: &str = concat!(
    "process rf (cmd: buffer in u32, rd: buffer out u32)\n",
    "  var vals: #[impl(bram)] [u32;32]\n",
    "  loop\n",
    "    let c = @rcv(cmd)\n",
    "    let a: u5 = c[4..0]\n",
    "    let b: u5 = c[9..5]\n",
    "    vals[a] = c\n",
    "    vals[b] = 32'd9\n",
    "    let x = vals[a]\n",
    "    @send(rd, x)\n",
);

#[test]
fn without_the_flag_the_ports_are_emitted_as_written() {
    // Which is what an ASIC memory compiler wants: a real two-write cell is
    // smaller and faster than anything built out of one-write blocks.
    let v = compile(TWO_WRITE_BRAM);
    assert!(v.contains("reg [31:0] vals [0:31];"), "{}", v);
    assert!(v.contains("vals[a] <= cmd_item;"), "{}", v);
    assert!(v.contains("vals[b] <= 32'd9;"), "{}", v);
    assert!(!v.contains("vals_lvt"), "{}", v);
}

#[test]
fn the_flag_builds_banks_and_a_live_value_table() {
    let v = compile_lvt(TWO_WRITE_BRAM);
    // One bank per write port, replicated per read port.
    assert!(v.contains("reg [31:0] vals_b0r0 [0:31];"), "{}", v);
    assert!(v.contains("reg [31:0] vals_b1r0 [0:31];"), "{}", v);
    // Each write goes to its own bank and stamps the table.
    assert!(v.contains("vals_b0r0[a] <= cmd_item;"), "{}", v);
    assert!(v.contains("vals_lvt[a] <= 1'd0;"), "{}", v);
    assert!(v.contains("vals_b1r0[b] <= 32'd9;"), "{}", v);
    assert!(v.contains("vals_lvt[b] <= 1'd1;"), "{}", v);
    // The read fetches every bank and the table on one enable, and the table
    // picks which answer is current.
    assert!(v.contains("vals_lvt_q0 <= vals_lvt["), "{}", v);
    assert!(
        v.contains("assign vals_q = (vals_lvt_q0 == 1'd0) ? vals_b0r0_q : vals_b1r0_q;"),
        "{}",
        v
    );
    // The table is the only array reset: resetting a bank is what stops a
    // block RAM being inferred, which is the whole point of doing this.
    assert!(v.contains("vals_lvt[vals_ix] <= 1'd0;"), "{}", v);
    assert!(!v.contains("vals_b0r0[vals_ix]"), "{}", v);
}

#[test]
fn the_flag_replicates_a_bank_per_read_port() {
    let v = compile_lvt(concat!(
        "sequence s (req: buffer in u32, dout: buffer out u32)\n",
        "  var vals: #[impl(bram)] [u32;32]\n",
        "  let c = @rcv(req)\n",
        "  vals[c[4..0]] = c\n",
        "  vals[c[9..5]] = 32'd9\n",
        "  let x = vals[c[14..10]]\n",
        "  let y = vals[c[19..15]]\n",
        "  |||\n",
        "  @send(dout, x + y)\n",
    ));
    for bank in 0..2 {
        for read in 0..2 {
            let arr = format!("vals_b{}r{}", bank, read);
            assert!(v.contains(&format!("reg [31:0] {} [0:31];", arr)), "{}: {}", arr, v);
        }
    }
    assert!(v.contains("assign vals_q0 = "), "{}", v);
    assert!(v.contains("assign vals_q1 = "), "{}", v);
}

#[test]
fn the_flag_leaves_a_single_write_bram_alone() {
    // One write port needs no banking: several reads of one write stream is a
    // replication every FPGA synthesizer already does by itself.
    let v = compile_lvt(LOOKUP);
    assert!(!v.contains("table__lvt"), "{}", v);
    assert!(v.contains("if (in_s1) table__q <= table_[a_r];"), "{}", v);
}

#[test]
fn the_flag_leaves_a_lutram_alone() {
    // It names `bram`, and distributed RAM is a different primitive with a
    // different answer to the same question.
    let v = compile_lvt(concat!(
        "process p (req: buffer in u4, dout: buffer out u32)\n",
        "  var cells: #[impl(lutram)] [u32;16] = @zeroed()\n",
        "  loop\n",
        "    let a = @rcv(req)\n",
        "    cells[a] = 32'd7\n",
        "    cells[a + 4'd1] = 32'd9\n",
        "    @send(dout, 32'd0)\n",
    ));
    assert!(!v.contains("cells_lvt"), "{}", v);
    assert!(v.contains("2 sync write ports"), "{}", v);
}
