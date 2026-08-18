
use std::collections::HashSet;

use crate::parse::{
    ProcessDecl,
    PrecResInnerStmt,
    VarDeclStmt,
    PrecTypeExpr,
    PrecResExpr,
    Literal,
    ForLoopStmt,
    LoopStmt,
    CallStmt,
    MatchStmt,
    MatchArm,
    StructDecl,
    EnumDecl,
    SequenceDecl,
    PrecSeqInnerStmt,
    FunctionDecl,
    anumspan_to_str
};
use crate::lex::{
    ArgTypeQualifier,
    BindingPattern
};

pub fn check_scope_in_struct(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    struct_ref: &StructDecl
) {
    let mut field_names = HashSet::new();
    for field in &struct_ref.fields {
        let fname = anumspan_to_str(&field.name);
        let new = field_names.insert(fname);
        if !new {
            let sname = anumspan_to_str(&struct_ref.name);
            aggregated_errs.push(format!("field named {} of {} is duplicated", fname, sname))
        }
        check_scope_in_type(aggregated_names, aggregated_errs, &field.field_type)
    }
} 
pub fn check_scope_in_enum(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    enum_ref: &EnumDecl
) {
    let mut variant_names = HashSet::new();
    for variant in &enum_ref.variants {
        let vname = anumspan_to_str(&variant.name);
        let is_new = variant_names.insert(vname);
        if !is_new {
            let ename = anumspan_to_str(&enum_ref.name);
            aggregated_errs.push(format!("variant named {} of {} is duplicated", vname, ename))
        }
        if let Some(payload) = &variant.payload {
            check_scope_in_type(aggregated_names, aggregated_errs, payload)
        }
    }
} 

pub fn check_scope_in_process(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    process: &ProcessDecl,
) {
    let mut local_names = aggregated_names.clone();
    for arg in &process.args.entries {
        match arg.qualifier {
            ArgTypeQualifier::Out | ArgTypeQualifier::Inout => {
                aggregated_errs.push("Invalid parameter qualifier in process".to_string());
            },
            _ => ()
        }
        check_scope_in_type(aggregated_names, aggregated_errs, &arg.type_expr);
        let arg_name = anumspan_to_str(&arg.arg_name);
        local_names.insert(arg_name);
    }

    check_scope_in_stmts(aggregated_errs, &mut local_names, &process.body, false);
}

pub fn check_scope_in_sequence(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    sequence: &SequenceDecl,
) {
    let mut local_names = aggregated_names.clone();
    for arg in &sequence.args.entries {
        match arg.qualifier {
            ArgTypeQualifier::Out | ArgTypeQualifier::Inout => {
                aggregated_errs.push("Invalid parameter qualifier in process".to_string());
            },
            _ => ()
        }
        check_scope_in_type(aggregated_names, aggregated_errs, &arg.type_expr);
        let arg_name = anumspan_to_str(&arg.arg_name);
        local_names.insert(arg_name);
    }

    for top_stmt in &sequence.body {
        match top_stmt {
            PrecSeqInnerStmt::SegmentSeparator => (),
            PrecSeqInnerStmt::Stmt(stmt) => {
                check_scope_in_stmt(aggregated_errs, aggregated_names, stmt, false);
            },
        }
    }
}

pub fn check_scope_in_function(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    function: &FunctionDecl,
) {
    let mut local_names = aggregated_names.clone();
    for arg in &function.args.entries {
        check_scope_in_type(aggregated_names, aggregated_errs, &arg.type_expr);
        let arg_name = anumspan_to_str(&arg.arg_name);
        local_names.insert(arg_name);
    }

    check_scope_in_stmts(aggregated_errs, &mut local_names, &function.body, false);
}

pub fn check_scope_in_stmts(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    stmts: &Vec<PrecResInnerStmt>,
    in_loop: bool,
) {
    let mut collected_locals = aggregated_names.clone();
    for item in stmts {
        check_scope_in_stmt(aggregated_errs, &mut collected_locals, item, in_loop)
    }
}

pub fn check_scope_in_stmt(
    aggregated_errs: &mut Vec<String>,
    aggregated_names: &mut HashSet<&str>,
    stmt: &PrecResInnerStmt,
    in_loop: bool
) {
    match stmt {
        PrecResInnerStmt::VarDecl(stmt) => {
            check_scope_in_var(stmt, aggregated_names, aggregated_errs);
            let name = anumspan_to_str(&stmt.name);
            aggregated_names.insert(name);
        },
        PrecResInnerStmt::MatchStmt(match_stmt) => {
            check_scope_in_match(match_stmt, aggregated_names, aggregated_errs, in_loop);
        },
        PrecResInnerStmt::CallStmt(call_stmt) => {
            check_scope_in_call(call_stmt, aggregated_names, aggregated_errs);
        },
        PrecResInnerStmt::IfThenElse(itestmt) => {
            check_scope_in_expr(&itestmt.condition, aggregated_names, aggregated_errs, false);
            check_scope_in_expr(&itestmt.then_case, aggregated_names, aggregated_errs, true);
            match &itestmt.else_case {
                Some(else_case) => {
                    check_scope_in_expr(else_case, aggregated_names, aggregated_errs, in_loop);
                },
                None => (),
            }
        },
        PrecResInnerStmt::AssignStmt(assign_stmt) => {
            check_scope_in_expr(&assign_stmt.rvalue, aggregated_names, aggregated_errs, false);
            let mut ok = true;
            check_scope_in_rval_form(&assign_stmt.lvalue, aggregated_names, aggregated_errs, &mut ok);
            if !ok {
                aggregated_errs.push(format!("invalid lvalue"))
            }
        },
        PrecResInnerStmt::TailVal(prec_res_expr) => {
            check_scope_in_expr(prec_res_expr, aggregated_names, aggregated_errs, false)
        },
        PrecResInnerStmt::Loop(loop_stmt) => {
            check_scope_in_loop(loop_stmt, aggregated_names, aggregated_errs)
        },
        PrecResInnerStmt::Break => {
            if !in_loop {
                aggregated_errs.push(format!("break not in loop"));
            }
        },
        PrecResInnerStmt::ForLoop(for_loop_stmt) => {
            check_scope_in_for_loop(&for_loop_stmt, aggregated_names, aggregated_errs)
        },
        PrecResInnerStmt::ReturnStmt(prec_res_expr) => {
            match prec_res_expr {
                Some(expr) => {
                    check_scope_in_expr(expr, aggregated_names, aggregated_errs, false)
                },
                None => (),
            }
        },
    }
}

fn check_scope_in_rval_form(
    expr:&PrecResExpr,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
    is_ok: &mut bool,
) {
    match expr {
        PrecResExpr::Ref(alphanum_span) => {
            let name = anumspan_to_str(alphanum_span);
            let known = aggregated_names.contains(name);
            if !known {
                aggregated_errs.push(format!("{} is unknown ident", name))
            }
        },
        PrecResExpr::Literal(_) => {
            *is_ok = false;
        },
        PrecResExpr::Builtin(_) => {
            *is_ok = false;
        },
        PrecResExpr::FieldAccess { base, field_name: _ } => {
            check_scope_in_rval_form(base, aggregated_names, aggregated_errs, is_ok);
        },
        PrecResExpr::Call { .. } => {
            *is_ok = false;
        },
        PrecResExpr::SubscriptAccess(subscript_access) => {
            check_scope_in_rval_form(&subscript_access.base, aggregated_names, aggregated_errs, is_ok);
            check_scope_in_expr(&subscript_access.index, aggregated_names, aggregated_errs, false);
        },
        PrecResExpr::Splice(_) => {
            *is_ok = false;
        },
        PrecResExpr::StmtBlock(_) => {
            *is_ok = false;
        },
        PrecResExpr::Span(_) => {
            *is_ok = false;
        },
    }
}
fn check_scope_in_match(
    stmt:&MatchStmt,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
    in_loop: bool,
) {
    let scrut_count = stmt.scrutinees.len();
    for case in &stmt.cases {
        let ok = case.binding_patterns.len() == scrut_count;
        if !ok {
            aggregated_errs.push(format!("invalid binding number"))
        }
    }
    for case in &stmt.cases {
        descend_into_patterns(case, aggregated_names, aggregated_errs, in_loop)
    }
}
fn descend_into_patterns(
    arm:&MatchArm,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
    in_loop: bool,
) {
    fn descend_into_pattern(binding: &BindingPattern, local_names:&mut HashSet<&str>) {
        match binding {
            BindingPattern::Alphanum(alphanum_span) => {
                let name = anumspan_to_str(&alphanum_span);
                local_names.insert(name);
            },
            BindingPattern::EnumCase { base:_, subbinding } => {
                if let Some(bind) = subbinding {
                    let name = anumspan_to_str(bind);
                    local_names.insert(name);
                }
            },
            BindingPattern::AnyOf(alternatives) => {
                for alt in alternatives {
                    descend_into_pattern(alt, local_names);
                }
            },
        }
    }
    let mut local_names = aggregated_names.clone();
    for binding in &arm.binding_patterns {
        descend_into_pattern(binding, &mut local_names);
    }
    check_scope_in_expr(&arm.rhs, &mut local_names, aggregated_errs, in_loop);
}
fn check_scope_in_for_loop(
    stmt:&ForLoopStmt,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
) {
    check_scope_in_expr(&stmt.target, aggregated_names, aggregated_errs, false);

    let mut local_names = aggregated_names.clone();
    let name = anumspan_to_str(&stmt.binding);
    local_names.insert(name);

    check_scope_in_expr(&stmt.body, &mut local_names, aggregated_errs, true);
}
fn check_scope_in_call(
    stmt:&CallStmt,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
) {
    match &stmt.base {
        PrecResExpr::Builtin(_) => (),
        PrecResExpr::Ref(nm) => {
            let name = anumspan_to_str(nm);
            let known = aggregated_names.contains(name);
            if !known {
                aggregated_errs.push(format!("{} is unknown function", name));
            }
        },
        PrecResExpr::Literal(_) |
        PrecResExpr::FieldAccess { base:_, field_name:_ } |
        PrecResExpr::Call { base:_, args:_ } |
        PrecResExpr::SubscriptAccess(_) |
        PrecResExpr::Splice(_) |
        PrecResExpr::StmtBlock(_) |
        PrecResExpr::Span(_) => {
            aggregated_errs.push(format!("unsupported call target"))
        },
    }
}
fn check_scope_in_loop(
    stmt:&LoopStmt,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
) {
    check_scope_in_expr(&stmt.repeat_expr, aggregated_names, aggregated_errs, true);
}

fn check_scope_in_var(
    stmt:&VarDeclStmt,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
) {
    match &stmt.ty_expr {
        Some(expr) => {
            check_scope_in_type(aggregated_names,aggregated_errs, expr)
        },
        None => (),
    }
    match &stmt.assign_val {
        Some(expr) => {
            check_scope_in_expr(expr, aggregated_names, aggregated_errs, false)
        },
        None => (),
    }
}

fn check_scope_in_type(
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
    expr:&PrecTypeExpr
) {
    fn validate_type_ident(
        iden: &str,
        aggregated_names: &mut HashSet<&str>,
        aggregated_errs: &mut Vec<String>,
    ) {
        if let [b'i', tail @ ..] = &iden.as_bytes()[..] {
            let nums = tail.iter().fold(true, |acc, el| acc & (*el as char).is_alphanumeric());
            if !nums {
                aggregated_errs.push(format!("invalid short bit integer type"))
            }
            return
        }
        let known = aggregated_names.contains(iden);
        if !known {
            aggregated_errs.push(format!("{} is unknown name", iden));
        }
    }
    match expr {
        PrecTypeExpr::Ident(alphanum_span) => {
            let name = anumspan_to_str(alphanum_span);
            validate_type_ident(name, aggregated_names, aggregated_errs);
        },
        PrecTypeExpr::Array(prec_type_expr, prec_res_expr) => {
            match prec_res_expr {
                PrecResExpr::Literal(Literal::IntLiteral { .. }) => {
                    check_scope_in_type(aggregated_names, aggregated_errs, prec_type_expr)
                },
                _ => {
                    aggregated_errs.push(format!("unsupported literal type"));
                }
            }
        },
    }
}

fn check_scope_in_expr(
    expr:&PrecResExpr,
    aggregated_names: &mut HashSet<&str>,
    aggregated_errs: &mut Vec<String>,
    in_loop: bool,
) {
    match expr {
        PrecResExpr::Ref(alphanum_span) => {
            let name = anumspan_to_str(alphanum_span);
            let known_name = aggregated_names.contains(name);
            if !known_name {
                aggregated_errs.push(format!("{} is unknown name", name));
            }
        },
        PrecResExpr::Literal(_) => (),
        PrecResExpr::Builtin(_) => (),
        PrecResExpr::FieldAccess { base, field_name: _ } => {
            check_scope_in_expr(base, aggregated_names, aggregated_errs, in_loop);
        },
        PrecResExpr::Call { base, args } => {
            check_scope_in_expr(base, aggregated_names, aggregated_errs, in_loop);
            for arg in args {
                check_scope_in_expr(arg, aggregated_names, aggregated_errs, in_loop);
            }
        },
        PrecResExpr::SubscriptAccess(stmt) => {
            check_scope_in_expr(&stmt.base, aggregated_names, aggregated_errs, in_loop);
            check_scope_in_expr(&stmt.index, aggregated_names, aggregated_errs, in_loop);
        },
        PrecResExpr::Splice(stmts) => {
            for stmt in stmts {
                check_scope_in_expr(stmt, aggregated_names, aggregated_errs, in_loop)
            }
        },
        PrecResExpr::StmtBlock(block) => {
            check_scope_in_stmts(aggregated_errs, aggregated_names, &block.components, in_loop);
        },
        PrecResExpr::Span(_) => (),
    }
}


// ---- scope checking ------------------------------------------------------
//
// These tests pin what this pass actually does. It is scope checking only --
// no types, no arity, no widths -- and it has real gaps, recorded below as
// tests that assert the CURRENT behaviour so that fixing one makes the test
// fail and forces the record to be updated.

#[cfg(test)]
fn check_process_src(src: &str) -> Vec<String> {
    use crate::lex::parse_top_level;
    use crate::parse::resolve_precedence_for_process;

    let range = src.as_bytes().as_ptr_range();
    let len = (range.end as usize) - (range.start as usize);
    let decls = unsafe { parse_top_level(range.start, len as u32) }
        .unwrap_or_else(|_| panic!("should parse:\n{}", src));
    let decl = match &decls[0] {
        crate::lex::TopLevelDecl::ProcessStmt(d) => d,
        other => panic!("expected a process, got {:?}", other),
    };
    let decl = unsafe { resolve_precedence_for_process(range.start, decl) }
        .unwrap_or_else(|_| panic!("should resolve:\n{}", src));

    let mut errs = Vec::new();
    let mut names = HashSet::new();
    check_scope_in_process(&mut errs, &mut names, &decl);
    errs
}

#[test]
fn an_undefined_name_is_reported() {
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x = nope\n",
    ));
    assert!(
        errs.iter().any(|e| e.contains("nope") && e.contains("unknown")),
        "{:?}",
        errs
    );
}

#[test]
fn a_process_parameter_is_in_scope() {
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x = arg1\n",
    ));
    assert!(errs.is_empty(), "{:?}", errs);
}

#[test]
fn break_outside_a_loop_is_reported() {
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  break\n",
    ));
    assert!(errs.iter().any(|e| e.contains("break not in loop")), "{:?}", errs);
}

#[test]
fn duplicate_struct_fields_are_reported() {
    use crate::lex::parse_top_level;
    use crate::parse::resolve_precedence_for_struct;
    let src = "struct Dup\n  a: i1\n  a: i1\n";
    let range = src.as_bytes().as_ptr_range();
    let len = (range.end as usize) - (range.start as usize);
    let decls = unsafe { parse_top_level(range.start, len as u32) }.expect("parses");
    let decl = match &decls[0] {
        crate::lex::TopLevelDecl::StructDecl(d) => d,
        other => panic!("expected a struct, got {:?}", other),
    };
    let decl = unsafe { resolve_precedence_for_struct(range.start, decl) }.expect("resolves");

    let mut errs = Vec::new();
    let mut names = HashSet::new();
    check_scope_in_struct(&mut errs, &mut names, &decl);
    assert!(errs.iter().any(|e| e.contains("duplicated")), "{:?}", errs);
}

#[test]
fn a_pipe_qualifier_on_a_process_output_is_rejected() {
    // desc.md:126 -- a process takes only direct parameters.
    let errs = check_process_src(concat!(
        "process Name (arg1: out i1)\n",
        "  return\n",
    ));
    assert!(
        errs.iter().any(|e| e.contains("Invalid parameter qualifier")),
        "{:?}",
        errs
    );
}

// ---- known gaps ----------------------------------------------------------
//
// Each of these asserts that NOTHING is reported. They are a ratchet: closing
// a gap makes the corresponding test fail, which is the signal to move it up
// into the section above.

#[test]
fn gap_redeclaration_is_not_reported() {
    // check_scope_in_stmt discards the HashSet::insert return value, so a
    // shadowing `let` is silently accepted.
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x = arg1\n",
        "  let x = arg1\n",
    ));
    assert!(errs.is_empty(), "redeclaration is now caught: {:?}", errs);
}

#[test]
fn gap_match_scrutinees_are_not_scope_checked() {
    // check_scope_in_match checks arm arity and descends into patterns, but
    // never walks `stmt.scrutinees`, so undefined names there go unreported.
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  match undefined_a, undefined_b\n",
        "    p, q => p\n",
    ));
    assert!(errs.is_empty(), "scrutinees are now checked: {:?}", errs);
}

#[test]
fn gap_break_inside_an_if_is_allowed_anywhere() {
    // sema.rs hardcodes `in_loop: true` for the then-branch of an `if`, so
    // this passes the break-outside-loop check despite there being no loop.
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  if arg1 then\n",
        "    break\n",
    ));
    assert!(errs.is_empty(), "if-then no longer fakes a loop: {:?}", errs);
}

#[test]
fn gap_no_type_checking_at_all() {
    // Widths are not checked here; that only happens once a declaration
    // reaches src/ty.rs via the IR. This pass would accept any nonsense.
    let errs = check_process_src(concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x: i8 = arg1\n",
    ));
    assert!(errs.is_empty(), "sema now type-checks: {:?}", errs);
}
