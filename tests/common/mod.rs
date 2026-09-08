//! The lowered-circuit simulator the behavioural tests share.
//!
//! Evaluates one `Module` -- values, registers and memories -- so a test can
//! drive a design cycle by cycle and read what comes out, without a Verilog
//! simulator. `from_module` exists for the modules the compiler writes rather
//! than the program: an adapter has no declaration to compile from.
#![allow(dead_code)]

use ddl::diag::{DiagSink, SourceMap};
use ddl::ir::{self, BinOp, CmpOp, Module, Op, UnOp};
use ddl::lex::TopLevelDecl;
use ddl::parse::*;
use std::collections::HashMap;

pub fn modules(src: &str) -> Vec<Module> {
    let map = SourceMap::new("regression.ddl", src);
    let parsed = ddl::driver::parse_source(&map).unwrap();
    let mut sink = DiagSink::new(&map);
    let (mut es, mut ss, mut fs, mut ps) = (vec![], vec![], vec![], vec![]);
    let mut sequences = vec![];
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
                TopLevelDecl::SequenceDecl(s) => {
                    sequences.push(resolve_precedence_for_sequence(map.base_ptr(), s).unwrap())
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
    for s in &sequences {
        if let Some(m) = ddl::ir_pipe::lower_sequence(&map, &syms, &bodies, s, &mut sink) {
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

pub fn mask(w: u32) -> u128 {
    u128::MAX >> (128 - w)
}
pub fn signed(v: u128, w: u32) -> i128 {
    ((v << (128 - w)) as i128) >> (128 - w)
}

pub struct Circuit {
    pub m: Module,
    pub regs: Vec<u128>,
    pub inputs: HashMap<String, u128>,
    pub memories: Vec<Vec<u128>>,
    pub read_outputs: Vec<Vec<u128>>,
    pub read_events: Vec<(usize, u128)>,
}
impl Circuit {
    pub fn new(src: &str, name: &str) -> Self {
        let m = modules(src).into_iter().find(|m| m.name == name).unwrap();
        let regs = m.regs.iter().map(|r| r.reset).collect();
        let memories = m.mems.iter().map(|mem| vec![mem.reset.unwrap_or(0); mem.len as usize]).collect();
        let read_outputs = m.mems.iter().map(|mem| vec![0; mem.read.len()]).collect();
        Self {
            m,
            regs,
            inputs: HashMap::new(),
            memories,
            read_outputs,
            read_events: vec![],
        }
    }
    pub fn set(&mut self, n: &str, v: u128) {
        self.inputs.insert(n.into(), v);
    }
    pub fn eval(&self) -> Vec<u128> {
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
                Op::MemRead { mem, addr } => self.memories[*mem as usize][v(*addr) as usize],
                Op::MemReadReg { mem, port } => self.read_outputs[*mem as usize][*port as usize],
            };
            vs[d.id.0 as usize] = result & mask(d.ty.bit_width());
        }
        vs
    }
    pub fn out(&self, n: &str) -> u128 {
        let vs = self.eval();
        let (_, v) = self
            .m
            .drivers
            .iter()
            .find(|(p, _)| self.m.port(*p).name == n)
            .unwrap();
        vs[v.0 as usize]
    }
    pub fn assertions_ok(&self) -> bool {
        let vs = self.eval();
        self.m.asserts.iter().all(|a| vs[a.cond.0 as usize] != 0)
    }
    pub fn tick(&mut self) {
        let vs = self.eval();
        // Synchronous reads observe memory before this edge's writes.
        for (ix, mem) in self.m.mems.iter().enumerate() {
            for (port, read) in mem.read.iter().enumerate() {
                if vs[read.en.0 as usize] != 0 {
                    let addr = vs[read.addr.0 as usize];
                    self.read_outputs[ix][port] = self.memories[ix][addr as usize];
                    self.read_events.push((ix, addr));
                }
            }
            for write in &mem.write {
                if vs[write.we.0 as usize] != 0 {
                    self.memories[ix][vs[write.addr.0 as usize] as usize] = vs[write.data.0 as usize];
                }
            }
        }
        self.regs = self.m.regs.iter().map(|r| vs[r.next.0 as usize]).collect();
    }
    pub fn collect(&mut self, cycles: usize, width: u32) -> Vec<u128> {
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


impl Circuit {
    /// A circuit over a module the compiler wrote, which has no source to
    /// compile from.
    pub fn from_module(m: Module) -> Self {
        let regs = m.regs.iter().map(|r| r.reset).collect();
        let memories = m
            .mems
            .iter()
            .map(|mem| vec![mem.reset.unwrap_or(0); mem.len as usize])
            .collect();
        let read_outputs = m.mems.iter().map(|mem| vec![0; mem.read.len()]).collect();
        Self { m, regs, inputs: HashMap::new(), memories, read_outputs, read_events: vec![] }
    }
}
