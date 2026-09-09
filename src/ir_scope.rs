//! Lexical identities for bindings before process or sequence lowering flattens blocks.
//!
//! Keep the first spelling for readable RTL; subsequent declarations of the
//! same name get compiler-owned identities. Initializers are resolved before
//! introducing their binding, so `var n = n + 1` can read an outer `n`.

use std::collections::{HashMap, HashSet};

use crate::diag::{DiagSink, SourceMap, Span};
use crate::lex::{AlphanumSpan, BindingPattern};
use crate::parse::{PrecResExpr, PrecResInnerStmt, anumspan_to_str};

type Scope = HashMap<String, AlphanumSpan>;

pub struct Scoped {
    pub stmts: Vec<PrecResInnerStmt>,
    pub origins: HashMap<usize, Span>,
}

struct Resolver<'a> {
    map: &'a SourceMap,
    origins: HashMap<usize, Span>,
    globals: HashSet<String>,
    used: HashSet<String>,
    locals: HashSet<String>,
    unresolved: Vec<AlphanumSpan>,
    next_name: usize,
}

impl Resolver<'_> {
    fn alias(&mut self, text: Box<str>, original: AlphanumSpan) -> AlphanumSpan {
        let span = AlphanumSpan::new(text.as_ref());
        self.origins
            .insert(span.byte_ptr as usize, self.map.span_of(&original));
        self.next_name += 1;
        span
    }
    fn bind(&mut self, span: &mut AlphanumSpan, scope: &mut Scope) {
        let original = anumspan_to_str(span).to_string();
        if original == "_" {
            return;
        }
        self.locals.insert(original.clone());
        if !self.used.insert(original.clone()) {
            let text = format!("@local{}_{}", self.next_name, original).into_boxed_str();
            *span = self.alias(text, span.clone());
        }
        scope.insert(original, span.clone());
    }

    fn pattern(&mut self, p: &mut BindingPattern, scope: &mut Scope) {
        match p {
            BindingPattern::Alphanum(n) => self.bind(n, scope),
            BindingPattern::EnumCase {
                subbinding: Some(n),
                ..
            } => self.bind(n, scope),
            BindingPattern::AnyOf(ps) => {
                for p in ps {
                    self.pattern(p, scope);
                }
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &mut PrecResExpr, scope: &Scope) {
        match e {
            PrecResExpr::Ref(n) => {
                if let Some(resolved) = scope.get(anumspan_to_str(n)) {
                    // Keep each reference's source anchor, including when no
                    // rename was needed. A declaration's anchor is not the
                    // location of every later use of the binding.
                    if anumspan_to_str(resolved) != anumspan_to_str(n) {
                        *n = self.alias(anumspan_to_str(resolved).into(), n.clone());
                    }
                } else {
                    self.unresolved.push(n.clone());
                }
            }
            PrecResExpr::Call { base, args } => {
                self.expr(base, scope);
                for arg in args {
                    self.expr(arg, scope);
                }
            }
            PrecResExpr::FieldAccess { base, .. } => self.expr(base, scope),
            PrecResExpr::SubscriptAccess(s) => {
                self.expr(&mut s.base, scope);
                self.expr(&mut s.index, scope);
            }
            PrecResExpr::Splice(parts) => {
                for p in parts {
                    self.expr(p, scope);
                }
            }
            PrecResExpr::Span(s) => {
                self.expr(&mut s.left, scope);
                self.expr(&mut s.right, scope);
            }
            PrecResExpr::StmtBlock(b) => self.block(&mut b.components, scope),
            _ => {}
        }
    }

    fn block(&mut self, stmts: &mut [PrecResInnerStmt], outer: &Scope) {
        let mut scope = outer.clone();
        for stmt in stmts {
            match stmt {
                PrecResInnerStmt::VarDecl(d) => {
                    if let Some(e) = &mut d.assign_val {
                        self.expr(e, &scope);
                    }
                    match &mut d.binding {
                        crate::lex::VarBindingKind::PlainName(n) => self.bind(n, &mut scope),
                        crate::lex::VarBindingKind::TuplePattern(ns) => {
                            for n in ns {
                                self.bind(n, &mut scope);
                            }
                        }
                    }
                }
                PrecResInnerStmt::AssignStmt(a) => {
                    self.expr(&mut a.lvalue, &scope);
                    self.expr(&mut a.rvalue, &scope);
                }
                PrecResInnerStmt::CallStmt(c) => {
                    self.expr(&mut c.base, &scope);
                    for arg in &mut c.args {
                        self.expr(arg, &scope);
                    }
                }
                PrecResInnerStmt::IfThenElse(i) => {
                    self.expr(&mut i.condition, &scope);
                    self.expr(&mut i.then_case, &scope);
                    if let Some(e) = &mut i.else_case {
                        self.expr(e, &scope);
                    }
                }
                PrecResInnerStmt::MatchStmt(m) => {
                    for e in &mut m.scrutinees {
                        self.expr(e, &scope);
                    }
                    for arm in &mut m.cases {
                        let mut arm_scope = scope.clone();
                        for p in &mut arm.binding_patterns {
                            self.pattern(p, &mut arm_scope);
                        }
                        self.expr(&mut arm.rhs, &arm_scope);
                    }
                }
                PrecResInnerStmt::Loop(l) => self.expr(&mut l.repeat_expr, &scope),
                PrecResInnerStmt::ForLoop(f) => {
                    self.expr(&mut f.target, &scope);
                    let mut body_scope = scope.clone();
                    self.bind(&mut f.binding, &mut body_scope);
                    self.expr(&mut f.body, &body_scope);
                }
                PrecResInnerStmt::TailVal(e) | PrecResInnerStmt::ReturnStmt(Some(e)) => {
                    self.expr(e, &scope);
                }
                _ => {}
            }
        }
    }
}

pub fn resolve(
    body: &[PrecResInnerStmt],
    globals: HashSet<String>,
    sink: &mut DiagSink,
) -> Option<Scoped> {
    // Errors from EARLIER declarations are not this one's failure: the sink
    // is shared by the whole compilation, so `has_errors` would make every
    // declaration after the first bad one return `None` without a reason.
    let errors_before = sink.error_mark();
    let mut r = Resolver {
        map: sink.map(),
        origins: HashMap::new(),
        used: globals.clone(),
        globals,
        locals: HashSet::new(),
        unresolved: Vec::new(),
        next_name: 0,
    };
    let mut stmts = body.to_vec();
    r.block(&mut stmts, &Scope::new());
    // An identifier belonging only to a sibling/ended scope must not become
    // visible just because the FSM happens to lower that scope first.
    for n in &r.unresolved {
        let name = anumspan_to_str(n);
        if r.locals.contains(name) && !r.globals.contains(name) {
            sink.err_at(n, format!("`{}` is not in scope here", name));
        }
    }
    if sink.errored_since(errors_before) {
        return None;
    }
    Some(Scoped {
        stmts,
        origins: r.origins,
    })
}
