//! Behavioral checks of the lowered circuit, independent of emitted names.
//! The companion Questa runner also checks the emitted Verilog itself.
use ddl::diag::{DiagSink, SourceMap};
use ddl::ir::{self, BinOp, CmpOp, Module, Op, UnOp};
use ddl::lex::TopLevelDecl;
use ddl::parse::*;
use std::collections::HashMap;

fn modules(src: &str) -> Vec<Module> {
    let map = SourceMap::new("regression.ddl", src);
    let parsed = ddl::driver::parse_source(&map).unwrap();
    let mut sink = DiagSink::new(&map);
    let (mut es, mut ss, mut fs, mut ps) = (vec![], vec![], vec![], vec![]);
    for d in &parsed.decls {
        // All ASTs borrow map until lowering finishes.
        unsafe {
            match d {
                TopLevelDecl::EnumDecl(e) => {
                    es.push(resolve_precedence_for_enum(map.base_ptr(), e).unwrap())
                }
                TopLevelDecl::StructDecl(s) => {
                    ss.push(resolve_precedence_for_struct(map.base_ptr(), s).unwrap())
                }
                TopLevelDecl::FunctionStmt(f) => {
                    fs.push(resolve_precedence_for_function(map.base_ptr(), f).unwrap())
                }
                TopLevelDecl::ProcessStmt(p) => {
                    ps.push(resolve_precedence_for_process(map.base_ptr(), p).unwrap())
                }
                _ => panic!("unsupported test declaration"),
            }
        }
    }
    let syms = ddl::symbols::build(&es, &ss, &fs, &mut sink);
    let bodies = fs
        .iter()
        .map(|f| (anumspan_to_str(&f.name).to_string(), f))
        .collect();
    let mut result = vec![];
    for f in &fs {
        if let Some(m) = ir::lower_function(&map, &syms, &bodies, f, &mut sink) {
            result.push(m);
        }
    }
    for p in &ps {
        if let Some(m) = ir::lower_process(&map, &syms, &bodies, p, &mut sink) {
            result.push(m);
        }
    }
    let diags = sink.into_diags();
    assert!(
        !diags
            .iter()
            .any(|d| d.severity == ddl::diag::Severity::Error),
        "{}",
        map.render_all(&diags)
    );
    result
}

fn mask(w: u32) -> u128 {
    u128::MAX >> (128 - w)
}
fn signed(v: u128, w: u32) -> i128 {
    ((v << (128 - w)) as i128) >> (128 - w)
}

struct Circuit {
    m: Module,
    regs: Vec<u128>,
    inputs: HashMap<String, u128>,
}
impl Circuit {
    fn new(src: &str, name: &str) -> Self {
        let m = modules(src).into_iter().find(|m| m.name == name).unwrap();
        let regs = m.regs.iter().map(|r| r.reset).collect();
        Self {
            m,
            regs,
            inputs: HashMap::new(),
        }
    }
    fn set(&mut self, n: &str, v: u128) {
        self.inputs.insert(n.into(), v);
    }
    fn eval(&self) -> Vec<u128> {
        let mut vs = vec![0u128; self.m.values.len()];
        for d in &self.m.values {
            let v = |id: ir::ValueId| vs[id.0 as usize];
            let width = |id| self.m.value(id).ty.bit_width();
            let result = match &d.op {
                Op::Port(p) => *self.inputs.get(&self.m.port(*p).name).unwrap_or(&0),
                Op::RegRead(i) => self.regs[*i as usize],
                Op::Const(c) => *c,
                Op::Bin { op, lhs, rhs } => {
                    let (a, b) = (v(*lhs), v(*rhs));
                    match op {
                        BinOp::Add => a.wrapping_add(b),
                        BinOp::Sub => a.wrapping_sub(b),
                        BinOp::Mul => a.wrapping_mul(b),
                        BinOp::Div => a.checked_div(b).unwrap_or(0),
                        BinOp::Mod => a.checked_rem(b).unwrap_or(0),
                        BinOp::And => a & b,
                        BinOp::Or => a | b,
                        BinOp::Xor => a ^ b,
                        BinOp::Shl => {
                            if b < 128 {
                                a << b
                            } else {
                                0
                            }
                        }
                        BinOp::Shr if self.m.value(*lhs).ty.is_signed() => {
                            (signed(a, width(*lhs)) >> b.min(127)) as u128
                        }
                        BinOp::Shr => {
                            if b < 128 {
                                a >> b
                            } else {
                                0
                            }
                        }
                    }
                }
                Op::Cmp { op, lhs, rhs } => {
                    let ord = if self.m.value(*lhs).ty.is_signed() {
                        signed(v(*lhs), width(*lhs)).cmp(&signed(v(*rhs), width(*rhs)))
                    } else {
                        v(*lhs).cmp(&v(*rhs))
                    };
                    (match op {
                        CmpOp::Eq => ord.is_eq(),
                        CmpOp::Ne => !ord.is_eq(),
                        CmpOp::Lt => ord.is_lt(),
                        CmpOp::Gt => ord.is_gt(),
                        CmpOp::Le => !ord.is_gt(),
                        CmpOp::Ge => !ord.is_lt(),
                    }) as u128
                }
                Op::Un { op, arg } => match op {
                    UnOp::BitNot => !v(*arg),
                    UnOp::Neg => 0u128.wrapping_sub(v(*arg)),
                    UnOp::LogNot => (v(*arg) == 0) as u128,
                },
                Op::Slice { arg, lo, .. } => v(*arg) >> lo,
                Op::DynSlice { arg, base, .. } => v(*arg).checked_shr(v(*base) as u32).unwrap_or(0),
                Op::Concat(parts) => parts
                    .iter()
                    .fold(0u128, |a, p| a.checked_shl(width(*p)).unwrap_or(0) | v(*p)),
                Op::Repeat { arg, times } => (0..*times).fold(0u128, |a, _| {
                    a.checked_shl(width(*arg)).unwrap_or(0) | v(*arg)
                }),
                Op::SExt { arg, .. } => signed(v(*arg), width(*arg)) as u128,
                Op::ZExt { arg, .. } | Op::Trunc { arg, .. } | Op::Cast { arg } => v(*arg),
                Op::Mux {
                    cond,
                    then_val,
                    else_val,
                } => v(if v(*cond) != 0 { *then_val } else { *else_val }),
                Op::Case {
                    scrutinee,
                    arms,
                    default,
                } => v(arms
                    .iter()
                    .find(|(labels, _)| labels.contains(&v(*scrutinee)))
                    .map(|(_, x)| *x)
                    .unwrap_or(*default)),
                other => panic!("unsupported test operation: {other:?}"),
            };
            vs[d.id.0 as usize] = result & mask(d.ty.bit_width());
        }
        vs
    }
    fn out(&self, n: &str) -> u128 {
        let vs = self.eval();
        let (_, v) = self
            .m
            .drivers
            .iter()
            .find(|(p, _)| self.m.port(*p).name == n)
            .unwrap();
        vs[v.0 as usize]
    }
    fn assertions_ok(&self) -> bool {
        let vs = self.eval();
        self.m.asserts.iter().all(|a| vs[a.cond.0 as usize] != 0)
    }
    fn tick(&mut self) {
        let vs = self.eval();
        self.regs = self.m.regs.iter().map(|r| vs[r.next.0 as usize]).collect();
    }
    fn collect(&mut self, cycles: usize, width: u32) -> Vec<u128> {
        let mut result = vec![];
        let mut r = 0;
        for cycle in 0..cycles {
            // Irregular backpressure, including more than a FIFO's capacity.
            if cycle % 11 >= 5 && self.out("o_wsalt") != r {
                let ix = (r ^ (r >> 1)) & 1;
                result.push((self.out("o_data") >> (ix as u32 * width)) & mask(width));
                r ^= if ix == 0 { 1 } else { 2 };
                self.set("o_rsalt", r);
            }
            assert!(self.assertions_ok());
            self.tick();
        }
        result
    }
}

#[test]
fn empty_nonblocking_input_never_advances_even_beside_a_barrier() {
    let mut c = Circuit::new(include_str!("probes/p1.ddl"), "p1");
    c.set("a_data", 42);
    for _ in 0..4 {
        c.tick();
        assert_eq!(c.out("b_rsalt"), 0);
    }
    c.set("a_wsalt", 1);
    assert_eq!(c.collect(60, 8), [42]);
    assert_eq!(c.out("b_rsalt"), 0);
}

#[test]
fn a_nonblocking_success_waits_for_the_enclosing_barrier_to_commit() {
    let mut c = Circuit::new(include_str!("probes/p1.ddl"), "p1");
    c.set("a_data", 42);
    c.set("b_data", 7);
    c.set("b_wsalt", 1);
    for _ in 0..12 {
        c.tick();
        assert_eq!(c.out("b_rsalt"), 0);
    }
    c.set("a_wsalt", 1);
    assert_eq!(c.collect(80, 8), [49]);
    assert_eq!(c.out("b_rsalt"), 1);
}

#[test]
fn a_conditional_drop_in_a_blocking_state_only_transfers_on_its_path() {
    let src = "process p (a: buffer in u1, b: buffer in u8, o: buffer out u1)\n  loop\n    let take = @rcv(a)\n    var got: u1 = 1'b0\n    if take then\n      got = @drop(b)\n    @send(o, got)\n";
    for take in [0, 1] {
        let mut c = Circuit::new(src, "p");
        c.set("a_wsalt", 1);
        c.set("a_data", take);
        c.set("b_wsalt", 1);
        assert_eq!(c.collect(80, 1), [take]);
        assert_eq!(c.out("b_rsalt"), take);
    }
}

#[test]
fn polling_empty_and_full_buffers_preserves_their_salts_and_entries() {
    let src = include_str!("probes/p5.ddl");
    let mut a = Circuit::new(src, "p5a");
    a.set("src_wsalt", 1);
    a.set("src_data", 42);
    let mut b = Circuit::new(src, "p5b");
    b.set("src_wsalt", 1);
    b.set("src_data", 42);
    b.set("dst_rsalt", 3);
    for _ in 0..20 {
        a.tick();
        b.tick();
        assert_eq!(a.out("other_rsalt"), 0);
        assert_eq!(b.out("dst_wsalt"), 0);
        assert_eq!(b.out("dst_data"), 0);
    }
    a.set("other_wsalt", 1);
    b.set("dst_rsalt", 0);
    for _ in 0..5 {
        a.tick();
        b.tick();
    }
    assert_eq!(a.out("other_rsalt"), 1);
    assert_eq!(b.out("dst_wsalt"), 1);
}

#[test]
fn a_taken_break_skips_the_later_send() {
    let mut c = Circuit::new(include_str!("probes/p5.ddl"), "p5c");
    c.set("src_wsalt", 1);
    c.set("src_data", 42);
    c.set("other_wsalt", 1);
    for _ in 0..20 {
        c.tick();
        assert_eq!(c.out("dst_wsalt"), 0);
    }
    assert_eq!(c.out("other_rsalt"), 1);
}

#[test]
fn narrow_payload_roundtrips_every_byte() {
    let src = include_str!("probes/p6.ddl");
    let mut pack = Circuit::new(src, "pack_small");
    let mut unpack = Circuit::new(src, "unpack");
    for x in 0..256 {
        pack.set("x", x);
        unpack.set("e", pack.out("o"));
        assert_eq!(unpack.out("o"), x);
    }
}

#[test]
fn an_assertion_checks_only_the_executing_match_arm() {
    let src = include_str!("probes/p8.ddl");
    let mut c = Circuit::new(src, "p8");
    c.set("q_wsalt", 1);
    c.set("q_data", 511);
    for _ in 0..20 {
        assert!(c.assertions_ok());
        c.tick();
    }
    assert_eq!(c.out("p_data") & 255, 255);
    let mut bad = Circuit::new(src, "p8");
    bad.set("q_wsalt", 1);
    bad.set("q_data", 255);
    bad.tick();
    assert!(!bad.assertions_ok(), "Rd8(255) must still fail");
}

#[test]
fn assertions_in_inlined_branch_conditions_are_execution_guarded_too() {
    let src = "fun permitted (x: u8, o: out u1)\n  @fatal(x != 8'hff, \"poison\")\n  o = x[0]\nprocess p (src: buffer in u8, o: buffer out u8)\n  loop\n    let x = @rcv(src)\n    if permitted(x) then\n      @send(o, x)\n";
    let mut c = Circuit::new(src, "p");
    c.set("src_data", 255);
    for _ in 0..8 {
        assert!(c.assertions_ok());
        c.tick();
    }
    c.set("src_wsalt", 1);
    assert!(
        !c.assertions_ok(),
        "the check must execute when the input arrives"
    );
    c.set("src_data", 1);
    assert_eq!(c.collect(80, 8), [1]);
}

#[test]
fn sends_and_branches_observe_source_order_under_backpressure() {
    let mut c = Circuit::new(include_str!("probes/p9.ddl"), "p9");
    assert_eq!(c.collect(200, 4), (0..8).collect::<Vec<_>>());
}

#[test]
fn locals_survive_waits_reinitialize_on_reentry_and_shadow_independently() {
    let mut c = Circuit::new(include_str!("probes/lifetimes.ddl"), "lifetimes");
    c.set("src_wsalt", 3);
    c.set("src_data", (4 << 8) | 1);
    assert_eq!(c.collect(300, 8), [1, 2, 3, 100, 101, 4, 9, 4, 5, 6, 7, 9]);
}

#[test]
fn an_inner_initializer_reads_the_outer_binding_and_reset_restarts_scopes() {
    let src = "process p (o: buffer out u8)\n  var n: u8 = 8'd10\n  loop\n    loop\n      var n: u8 = n + 8'd1\n      @send(o, n)\n      break\n    @send(o, n)\n    break\n";
    let mut c = Circuit::new(src, "p");
    assert_eq!(c.collect(100, 8), [11, 10]);
    c.regs = c.m.regs.iter().map(|r| r.reset).collect();
    c.inputs.clear();
    assert_eq!(c.collect(100, 8), [11, 10]);
}

#[test]
fn a_typed_mutable_receive_checks_its_initializer_type() {
    let map = SourceMap::new(
        "type.ddl",
        "process p (i: buffer in u8, o: buffer out u4)\n  loop\n    var n: u4 = @rcv(i)\n    @send(o, n)\n",
    );
    let errs = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(
        map.render_all(&errs)
            .contains("declared `u4` but the pipe carries `u8`")
    );
}

#[test]
fn a_mutable_receive_binding_survives_a_nested_loop() {
    let mut c = Circuit::new(include_str!("probes/lifetimes.ddl"), "received_local");
    c.set("src_wsalt", 3);
    c.set("src_data", (2 << 8) | 3);
    assert_eq!(c.collect(200, 8), [3, 2, 1, 2, 1]);
}

#[test]
fn a_binding_cannot_escape_its_lexical_scope() {
    let map = SourceMap::new(
        "scope.ddl",
        "process p (o: buffer out u8)\n  loop\n    loop\n      var n: u8 = 8'd1\n      @send(o, n)\n      break\n    @send(o, n)\n",
    );
    let errs = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errs).contains("`n` is not in scope here"));
}

#[test]
fn match_payloads_and_mutable_locals_are_separate_in_sibling_arms() {
    let mut c = Circuit::new(include_str!("probes/arm_scopes.ddl"), "arm_scopes");
    c.set("src_wsalt", 3);
    c.set("src_data", (0x11234 << 17) | 0xaa00);
    assert_eq!(c.collect(120, 16), [0xaa, 0xab, 0x1234, 0x1236]);
}

#[test]
fn a_shadowed_reference_keeps_its_own_source_location() {
    let map = SourceMap::new(
        "shadow.ddl",
        "process p (o: buffer out u16)\n  var n: u8 = 8'd1\n  loop\n    loop\n      var n: u8 = 8'd2\n      @send(o, n)\n      break\n",
    );
    let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
    assert!(map.render_all(&errors).contains("--> shadow.ddl:6:"));
}

#[test]
fn selection_bases_remain_signals_and_register_names_are_unique() {
    let emit = |src| {
        ddl::driver::compile_to_verilog(&SourceMap::new("test.ddl", src), &Default::default())
            .unwrap()
    };
    let selects = emit(include_str!("probes/p7.ddl"));
    assert!(!selects.contains("[7:0][7]"));
    assert!(!selects.contains("(v >> k)["));
    let names = emit(include_str!("probes/p11.ddl"));
    assert!(names.contains("reg [7:0] ans_data_1;"));
    assert!(names.contains("assign ans_data = {ans_e1, ans_e0};"));
}

#[test]
fn receive_and_drop_ignore_unrelated_output_backpressure() {
    for operation in ["let took = @drop(src)", "let (x, took) = @try_rcv(src)"] {
        for available in [0, 1] {
            let source = format!(
                "process p (src: buffer in u8, blocked: buffer out u8, observed: port out u1)\n  loop\n    {operation}\n    @try_send(observed, took)\n"
            );
            let mut c = Circuit::new(&source, "p");
            c.set("src_wsalt", available);
            c.set("src_data", 42);
            c.set("blocked_rsalt", 3); // wsalt=0 means full.
            assert_eq!(c.out("observed"), available);
            c.tick();
            assert_eq!(c.out("src_rsalt"), available);
            assert_eq!(c.out("blocked_wsalt"), 0);
        }
    }
}

#[test]
fn peek_only_and_unrequested_inputs_never_consume() {
    for available in [0, 1] {
        let source = "process p (src: buffer in u8, unused: buffer in u8, observed: port out u1)\n  loop\n    let (x, present) = @peek(src)\n    @try_send(observed, present)\n";
        let mut c = Circuit::new(source, "p");
        c.set("src_wsalt", available);
        c.set("unused_wsalt", 1);
        for _ in 0..12 {
            assert_eq!(c.out("observed"), available);
            c.tick();
            assert_eq!(c.out("src_rsalt"), 0);
            assert_eq!(c.out("unused_rsalt"), 0);
        }
    }
}

#[test]
fn polling_peek_send_drop_forwards_each_item_once_under_backpressure() {
    let mut c = Circuit::new(include_str!("probes/communication.ddl"), "polling_forward");
    c.set("unrelated_rsalt", 3);
    let (mut w, mut r, mut produced, mut pair) = (0u128, 0u128, 0u128, 0u128);
    let mut received = Vec::new();
    for cycle in 0..400 {
        if produced < 64 && w != ((!c.out("src_rsalt")) & 3) && cycle % 7 != 0 {
            let idx = (w ^ (w >> 1)) & 1;
            pair = (pair & !(255 << (idx * 8))) | (produced << (idx * 8));
            produced += 1;
            w ^= if idx == 0 { 1 } else { 2 };
        }
        c.set("src_wsalt", w);
        c.set("src_data", pair);
        if cycle % 11 >= 6 && c.out("o_wsalt") != r {
            let idx = (r ^ (r >> 1)) & 1;
            received.push((c.out("o_data") >> (idx * 8)) & 255);
            r ^= if idx == 0 { 1 } else { 2 };
        }
        c.set("o_rsalt", r);
        assert!(c.assertions_ok());
        c.tick();
        assert_eq!(c.out("unrelated_wsalt"), 0);
    }
    assert_eq!(received, (0..64).collect::<Vec<_>>());
    assert_eq!(c.out("src_rsalt"), w);
}

#[test]
fn a_finished_polling_process_stops_sends_and_execution_assertions() {
    let mut c = Circuit::new(include_str!("probes/communication.ddl"), "polling_once");
    c.set("src_wsalt", 1);
    c.set("src_data", 42);
    assert!(c.assertions_ok());
    c.tick();
    for _ in 0..12 {
        assert!(c.assertions_ok());
        assert_eq!(c.out("src_rsalt"), 1);
        assert_eq!(c.out("o_wsalt"), 1);
        assert_eq!(c.out("o_data") & 255, 42);
        c.tick();
    }
}

#[test]
fn independent_is_an_identifier_not_a_process_modifier() {
    use ddl::driver::compile_to_verilog;
    use ddl::verilog::EmitOptions;
    let ordinary = SourceMap::new(
        "name.ddl",
        "process independent (src: buffer in u8)\n  loop\n    let took = @drop(src)\n",
    );
    assert!(compile_to_verilog(&ordinary, &EmitOptions::default()).is_ok());
    let blocking = SourceMap::new(
        "mode.ddl",
        "process independent p (src: buffer in u8)\n  loop\n    let x = @rcv(src)\n",
    );
    assert!(compile_to_verilog(&blocking, &EmitOptions::default()).is_err());
}

#[test]
fn shared_branch_continuations_survive_every_predecessor() {
    for first in [false, true] {
        for second in [false, true] {
            for use_match in [false, true] {
                let next = if use_match {
                    "    match b\n      .Yes =>\n        @send(o, 8'd1)\n      .No =>\n        @send(o, 8'd2)\n"
                } else {
                    "    if b then\n      @send(o, 8'd1)\n    else\n      @send(o, 8'd2)\n"
                };
                let d_ty = if use_match { "choice" } else { "u1" };
                let src = format!("enum choice: u1\n  No\n  Yes\nprocess p (c: buffer in u1, d: buffer in {d_ty}, i: buffer in u8, o: buffer out u8)\n  loop\n    let a = @rcv(c)\n    let b = @rcv(d)\n    if a then\n      let x = @rcv(i)\n{next}");
                let mut c = Circuit::new(&src, "p");
                c.set("c_wsalt", 1); c.set("c_data", first as u128);
                c.set("d_wsalt", 1); c.set("d_data", second as u128);
                c.set("i_wsalt", 1);
                assert_eq!(c.collect(60, 8), [if second { 1 } else { 2 }]);
            }
        }
    }
}

#[test]
fn for_only_reads_cross_waits() {
    let mut c = Circuit::new(include_str!("probes/adversarial.ddl"), "for_read2");
    c.set("src_wsalt", 3); c.set("src_data", 0x0703);
    for _ in 0..8 { c.tick(); }
    c.set("other_wsalt", 1);
    for _ in 0..20 { c.tick(); }
    assert_eq!(c.out("dst_wsalt"), 1);
    assert_eq!(c.out("dst_data") & 255, 12);
}

#[test]
fn exclusive_channel_requests_mux_data_and_transfer_results() {
    for port in [false, true] {
        for flag in [0, 1] {
            for full in [false, true] {
                let kind = if port { "port" } else { "buffer" };
                let src = format!("process p (c: buffer in u1, o: {kind} out u8, success: port out u1)\n  loop\n    let (flag, present) = @peek(c)\n    var sent: u1 = 1'b0\n    if flag then\n      sent = @try_send(o, 8'd1)\n    else\n      sent = @try_send(o, 8'd2)\n    @try_send(success, sent)\n");
                let mut c = Circuit::new(&src, "p");
                c.set("c_wsalt", 1); c.set("c_data", flag);
                if !port { c.set("o_rsalt", if full {3} else {0}); }
                assert_eq!(c.out("success"), (port || !full) as u128);
                if port { assert_eq!(c.out("o"), if flag == 1 {1} else {2}); }
                c.tick();
                if !port {
                    assert_eq!(c.out("o_wsalt"), (!full) as u128);
                    if !full { assert_eq!(c.out("o_data") & 255, if flag == 1 {1} else {2}); }
                }
            }
        }
    }
}

#[test]
fn overlapping_requests_and_barrier_port_duplicates_are_rejected() {
    for body in [
        "    @try_send(o, 8'd1)\n    @try_send(o, 8'd2)\n",
        "    @try_send(o, 8'd1)\n    @send(o, 8'd2)\n",
        "    @send(o, 8'd1)\n    @try_send(o, 8'd2)\n",
    ] {
        let map = SourceMap::new("duplicate.ddl", format!("process p (o: port out u8)\n  loop\n{body}"));
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        assert!(map.render_all(&errors).contains("sent to more than once in one cycle"));
    }
}

#[test]
fn for_waits_and_breaks_have_an_explicit_diagnostic() {
    for op in ["@send(o, 8'd1)", "let x = @rcv(i)", "break", "let x = mem[0]"] {
        let memory = if op.contains("mem") { "  var mem: #[impl(bram)] [u8; 16]\n" } else { "" };
        let map = SourceMap::new("for_wait.ddl", format!("process p (i: buffer in u8, o: buffer out u8)\n{memory}  loop\n    for k in 0..2\n      {op}\n    @send(o, 8'd0)\n"));
        let errors = ddl::driver::compile_to_verilog(&map, &Default::default()).unwrap_err();
        let text = map.render_all(&errors);
        assert!(text.contains("a `for` body must be combinational"), "{text}");
    }
}
