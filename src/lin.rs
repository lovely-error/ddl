// SUPERSEDED by src/ir.rs, and not part of the compilation path.
//
// This was the original lowering: a flat `Vec<PrimAction>` with no control
// flow, which is why nine of its ten statement kinds are `todo!()` -- you
// cannot lower `if`, `loop`, `match` or `break` into a straight-line op list
// that has no branch instruction. `src/ir.rs` replaces it with typed values
// and an SSA join.
//
// It is kept for one reason: the channel operations sketched here (TrySend,
// BlockingSend, TryRecieve, BlockingRecieve) are the shape M3 needs when the
// FSM scheduler lands. Delete it once those move into `ir.rs`.
#![allow(dead_code)]

use std::collections::HashMap;

use crate::lex::{
    AlphanumSpan,
};
use crate::parse::{
    PrecTypeExpr,
    PrecArgDefTuple,
    ProcessDecl,
    PrecResInnerStmt,
    PrecResExpr,
    BuiltinOp,
    Literal,
    anumspan_to_str
};

#[derive(Debug)]
struct ProcessDef {
    pub name: AlphanumSpan,
    pub args: PrecArgDefTuple,
    pub body: Vec<PrimAction>
}

#[derive(Debug)]
struct StorageDecl {
    index: VarIndex,
    storage_type: Option<PrecTypeExpr>
}

#[derive(Debug)]
struct PrimAction {
    index: VarIndex,
    value: PrimOp
}

#[derive(Debug)]
enum PrimOp {
    Ref {
        ref_:AlphanumSpan
    },
    DeclStore {
        val_type: Option<PrecTypeExpr>
    },
    InlineVal {
        literal: Literal
    },
    Add {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Sub {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Mul {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Div {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Eq {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Shl {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Shr {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    And {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    Or {
        arg1: VarIndex,
        arg2: VarIndex,
    },
    BitInvert {
        arg1: VarIndex,
    },
    Mod {
        arg1: VarIndex,
    },
    TrySend {
        chan: VarIndex,
        val: VarIndex,
    },
    BlockingSend {
        chan: VarIndex,
        val: VarIndex,
    },
    TryRecieve {
        chan: VarIndex,
    },
    BlockingRecieve {
        chan: VarIndex,
    },
}

fn stmts_to_prim_op_seqv(
    ops: &mut Vec<PrimAction>,
    stmts: &Vec<PrecResInnerStmt>,
    index_gen: IndexGen,
    index_map: &mut HashMap<&str, VarIndex>,
) {
    for stmt in stmts {
        let _ = stmt_to_prim_op_seqv(ops, stmt, index_gen, index_map);
    }
}

fn stmt_to_prim_op_seqv(
    ops: &mut Vec<PrimAction>,
    stmt: &PrecResInnerStmt,
    index_gen: IndexGen,
    index_map: &mut HashMap<&str, VarIndex>,
) -> VarIndex {
    match stmt {
        PrecResInnerStmt::VarDecl(stmt) => {
            let name = anumspan_to_str(&stmt.name);
            if let Some(val) = &stmt.assign_val {
                let vix = expr_to_prim_op_seqv(ops, val, index_gen, index_map);
                index_map.insert(&name, vix);
                return vix
            } else {
                let ix = index_gen.next().unwrap();
                index_map.insert(&name, ix);
                ops.push(PrimAction { index: ix, value: PrimOp::DeclStore { val_type: stmt.ty_expr.clone() } });
                return ix
            }
        },
        PrecResInnerStmt::MatchStmt(_match_stmt) => todo!(),
        PrecResInnerStmt::CallStmt(_call_stmt) => todo!(),
        PrecResInnerStmt::IfThenElse(_itestmt) => todo!(),
        PrecResInnerStmt::AssignStmt(_assign_stmt) => todo!(),
        PrecResInnerStmt::TailVal(_prec_res_expr) => todo!(),
        PrecResInnerStmt::Loop(_loop_stmt) => todo!(),
        PrecResInnerStmt::Break => todo!(),
        PrecResInnerStmt::ForLoop(_for_loop_stmt) => todo!(),
        PrecResInnerStmt::ReturnStmt(_prec_res_expr) => todo!(),
    }
}

type IndexGen<'a> = &'a mut dyn Iterator<Item=VarIndex>;

#[derive(Debug, Clone, Copy)]
struct VarIndex(usize);

fn expr_to_prim_op_seqv(
    ops: &mut Vec<PrimAction>,
    expr: &PrecResExpr,
    index_gen: IndexGen,
    index_map: &mut HashMap<&str, VarIndex>
) -> VarIndex {
    match expr {
        PrecResExpr::Call { base, args } => {
            if let PrecResExpr::Builtin(builtin_op) = &**base {
                let mut args_ixs = Vec::new();
                for arg in args {
                    let ix = expr_to_prim_op_seqv(ops, arg, index_gen, index_map);
                    args_ixs.push(ix);
                }
                match builtin_op {
                    BuiltinOp::Add => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Add { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Sub => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Sub { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Mul => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Mul { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Div => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Div { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Eq => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Eq { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Shl => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Shl { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Shr => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Shr { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::And => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::And { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::Or => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Or { arg1: v1, arg2: v2 } });
                        return ix
                    },
                    BuiltinOp::BitInvert => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::BitInvert { arg1: v1 } });
                        return ix
                    },
                    BuiltinOp::Mod => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::Mod { arg1: v1 } });
                        return ix
                    },
                    BuiltinOp::Simd => {
                        todo!()
                    },
                    BuiltinOp::TrySend => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::TrySend { chan: v1, val: v2 }});
                        return ix
                    },
                    BuiltinOp::TryRecieve => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::TryRecieve { chan: v1 } });
                        return ix
                    },
                    BuiltinOp::BlockingSend => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        let v2 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::BlockingSend { chan: v1, val: v2 } });
                        return ix
                    },
                    BuiltinOp::BlockingRecieve => {
                        let ix = index_gen.next().unwrap();
                        let mut it = args_ixs.into_iter();
                        let v1 = it.next().unwrap();
                        ops.push(PrimAction { index: ix, value: PrimOp::BlockingRecieve { chan: v1 } });
                        return ix
                    },
                    // This lowering is superseded by src/ir.rs, which has a
                    // control-flow graph and typed values. The operators added
                    // with the new expression grammar are implemented only
                    // there.
                    _ => todo!(),
                }
                // op.push(PrimOp::DeclStorage(StorageDecl { index: (), storage_type: () }))
            } else {
                todo!()
            }
        },
        PrecResExpr::FieldAccess { base: _, field_name: _ } => todo!(),
        PrecResExpr::SubscriptAccess(_subscript_access) => todo!(),
        PrecResExpr::Literal(literal) => {
            let ix = index_gen.next().unwrap();
            ops.push(PrimAction { index: ix, value: PrimOp::InlineVal { literal: literal.clone() } });
            return ix
        },

        PrecResExpr::Ref(span) => {
            let ix = index_gen.next().unwrap();
            ops.push(PrimAction { index: ix, value: PrimOp::Ref { ref_: *span } });
            return ix
        },
        PrecResExpr::Builtin(_builtin_op) => todo!(),
        PrecResExpr::Splice(_prec_res_exprs) => todo!(),
        PrecResExpr::StmtBlock(_stmt_block) => todo!(),
        PrecResExpr::Span(_span) => todo!(),
    }
}

fn convert_process_body_to_prim_op_seqv(
    process: &ProcessDecl,
    index_gen: IndexGen,
    index_map: &mut HashMap<&str, VarIndex>,
) -> ProcessDef {
    let mut ops = Vec::new();
    stmts_to_prim_op_seqv(&mut ops, &process.body, index_gen, index_map);
    return ProcessDef { name: process.name, args: process.args.clone(), body: ops }
}


#[test]
fn a_builtin_call_lowers_to_a_prim_op() {
    // Pins the one path that works, so this file cannot rot unnoticed while
    // it waits to be folded into src/ir.rs.
    use crate::lex::parse_top_level;
    use crate::parse::resolve_precedence_for_process;

    let src = concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x : [[i1;2];2] = @send(arg1, 0)\n",
    );
    let range = src.as_bytes().as_ptr_range();
    let len = (range.end as usize) - (range.start as usize);
    let decls = unsafe { parse_top_level(range.start, len as u32) }.expect("parses");
    let decl = match &decls[0] {
        crate::lex::TopLevelDecl::ProcessStmt(d) => d,
        other => panic!("expected a process, got {:?}", other),
    };
    let decl = unsafe { resolve_precedence_for_process(range.start, decl) }.expect("resolves");

    let mut ig = core::iter::from_coroutine(
        #[coroutine]
        || {
            let mut ix = 0;
            loop {
                yield VarIndex(ix);
                ix += 1;
            }
        },
    );
    let mut index_map = HashMap::new();
    let def = convert_process_body_to_prim_op_seqv(&decl, &mut ig, &mut index_map);

    assert_eq!(crate::parse::anumspan_to_str(&def.name), "Name");
    assert!(
        def.body
            .iter()
            .any(|a| matches!(a.value, PrimOp::BlockingSend { .. })),
        "@send should lower to a BlockingSend: {:?}",
        def.body
    );
}
