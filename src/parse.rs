use crate::lex::{
    AlphanumSpan, AssignStmtKind, BasicInfixOp, BindingPattern, InfixExprComponent, InnerStmt,
    PostfixOp, RawArgDefTuple as RawArgTuple, RawExpr, RawProcessDecl, RawTypeExpr, StrLiteral, ArgTypeQualifier, RawFunctionDecl, RawSequenceDecl, RawSeqInnerStmt, RawStructDecl, RawEnumDecl, RawNum, UnaryOp, RawGraphDecl, RawGraphStmt
};


#[derive(Debug, Clone)]
pub enum PrecTypeExpr {
    Ident(AlphanumSpan),
    Array(Box<PrecTypeExpr>, PrecResExpr),
    /// `#[impl(lutram)] [T; n]`. The kind span is kept rather than resolved to
    /// an enum here so an unknown one reports at its own source position.
    MemArray { elem: Box<PrecTypeExpr>, len: PrecResExpr, kind: AlphanumSpan },
}

#[derive(Debug, Clone)]
pub struct PrecArgTupleEntry {
    pub arg_name: AlphanumSpan,
    pub qualifier: ArgTypeQualifier,
    pub type_expr: PrecTypeExpr,
    pub default: Option<PrecResExpr>,
}
/// `graph Name (ports)`, after type resolution.
#[derive(Debug, Clone)]
pub struct GraphDecl {
    pub name: AlphanumSpan,
    pub args: PrecArgDefTuple,
    pub body: Vec<GraphStmt>,
}

#[derive(Debug, Clone)]
pub enum GraphStmt {
    Pipe(GraphPipe),
    Instance(GraphInstance),
}

#[derive(Debug, Clone)]
pub struct GraphPipe {
    pub name: AlphanumSpan,
    /// Which word the `let` used, if any; lowering says what is wrong with it.
    pub said: crate::lex::PipeWord,
    pub ty: PrecTypeExpr,
}

#[derive(Debug, Clone)]
pub struct GraphInstance {
    pub module: AlphanumSpan,
    pub args: Vec<AlphanumSpan>,
}

#[derive(Debug, Clone)]
pub struct PrecArgDefTuple {
    pub entries: Vec<PrecArgTupleEntry>,
}

#[derive(Debug)]
pub struct ProcessDecl {
    pub name: AlphanumSpan,
    pub args: PrecArgDefTuple,
    pub body: Vec<PrecResInnerStmt>,
}

#[derive(Debug)]
pub struct FunctionDecl {
    pub name: AlphanumSpan,
    pub args: PrecArgDefTuple,
    pub body: Vec<PrecResInnerStmt>,
}


#[derive(Debug)]
pub struct SequenceDecl {
    pub name: AlphanumSpan,
    pub args: PrecArgDefTuple,
    pub body: Vec<PrecSeqInnerStmt>,
}

#[derive(Debug)]
pub struct StructDecl {
    pub name: AlphanumSpan,
    pub fields: Vec<StructField>,
}
#[derive(Debug)]
pub struct StructField {
    pub name: AlphanumSpan,
    pub field_type: PrecTypeExpr
}

#[derive(Debug)]
pub struct EnumDecl {
    pub name: AlphanumSpan,
    /// Explicit tag width from `enum Name: iN`, if written.
    pub tag_type: Option<PrecTypeExpr>,
    pub variants: Vec<EnumVariant>,
}
#[derive(Debug)]
pub struct EnumVariant {
    pub name: AlphanumSpan,
    /// Resolved but not yet folded; const evaluation happens in ty.rs.
    pub discriminant: Option<PrecResExpr>,
    pub payload: Option<PrecTypeExpr>,
}

#[derive(Debug)]
pub enum PrecSeqInnerStmt {
    SegmentSeparator,
    Stmt(PrecResInnerStmt)
}

#[derive(Debug)]
pub enum ExprInfixOp {
    Plus,  // +
    Minus, // -
    Mul,   // *
    Div,   // /
    Shl,   // <<
    Shr,   // >>
    Mod,
    And,
    Or,
    Eq,
}

#[derive(Debug, Clone)]
pub enum PrecResExpr {
    Ref(AlphanumSpan),
    Literal(Literal),
    Builtin(BuiltinOp),
    FieldAccess {
        base: Box<PrecResExpr>,
        field_name: AlphanumSpan,
    },
    Call {
        base: IndirectPrecResExpr,
        args: Vec<PrecResExpr>,
    },
    SubscriptAccess(Box<SubscriptAccess>),
    Splice(Vec<PrecResExpr>),
    StmtBlock(StmtBlock),
    Span(Box<Span>),
}
type IndirectPrecResExpr = Box<PrecResExpr>;

#[derive(Debug, Clone)]
pub struct Span {
    pub left: PrecResExpr,
    pub right: PrecResExpr,
}

#[derive(Debug, Clone)]
pub struct SubscriptAccess {
    pub base: PrecResExpr,
    pub index: PrecResExpr,
}

#[derive(Debug, Clone)]
pub enum Literal {
    /// `width` is `Some` only for a sized literal like `8'hFF`. An unsized
    /// literal takes its width from context, so the two cases must stay
    /// distinguishable all the way to the backend.
    IntLiteral {
        value: u128,
        width: Option<u32>,
    },
    FloatLiteral {
        whole: u64,
        frac: u64,
    },
    StrLiteral(StrLiteral),
}

#[derive(Debug, Clone)]
pub struct StmtBlock {
    pub components: Vec<PrecResInnerStmt>,
}

#[derive(Debug, Clone)]
pub enum PrecResInnerStmt {
    VarDecl(VarDeclStmt),
    MatchStmt(MatchStmt),
    CallStmt(CallStmt),
    IfThenElse(ITEStmt),
    AssignStmt(AssignStmt),
    TailVal(PrecResExpr),
    Loop(LoopStmt),
    Break,
    ForLoop(Box<ForLoopStmt>),
    ReturnStmt(Option<PrecResExpr>),
}

#[derive(Debug, Clone)]
pub struct ForLoopStmt {
    pub binding: AlphanumSpan,
    pub target: PrecResExpr,
    pub body: PrecResExpr,
}

#[derive(Debug, Clone)]
pub struct LoopStmt {
    pub repeat_expr: PrecResExpr,
}

#[derive(Debug, Clone)]
pub struct CallStmt {
    pub base: PrecResExpr,
    pub args: Vec<PrecResExpr>,
}

#[derive(Debug, Clone)]
pub struct AssignStmt {
    pub lvalue: PrecResExpr,
    pub rvalue: PrecResExpr,
    pub kind: AssignStmtKind,
}

#[derive(Debug, Clone)]
pub struct ITEStmt {
    pub condition: PrecResExpr,
    pub then_case: PrecResExpr,
    pub else_case: Option<PrecResExpr>,
}

#[derive(Debug, Clone)]
pub struct VarDeclStmt {
    pub is_mutable: bool,
    pub name: AlphanumSpan,
    /// The remaining names of a tuple binding; see `lex::VarDeclStmt`.
    pub rest: Vec<AlphanumSpan>,
    pub ty_expr: Option<PrecTypeExpr>,
    pub assign_val: Option<PrecResExpr>,
}

#[derive(Debug, Clone)]
pub struct MatchStmt {
    pub scrutinees: Vec<PrecResExpr>,
    pub cases: Vec<MatchArm>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub binding_patterns: Vec<BindingPattern>,
    pub rhs: PrecResExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinOp {
    Simd,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    Shl,
    Shr,
    // Bitwise.
    And,
    Or,
    Xor,
    BitInvert,
    // Comparison. All yield i1.
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    // Logical. Operands and result are i1.
    LogAnd,
    LogOr,
    LogNot,
    /// `@unreachable`, valid only as the body of a `match` catch-all: an
    /// assertion that no value reaches that arm.
    Unreachable,
    /// `if c then a else b`, desugared. Three operands: condition, then, else.
    Select,
    /// `@cast(x)`: reinterpret the bits as the type the context expects.
    Cast,
    // Unary arithmetic.
    Neg,
    // Width and signedness casts: @zext(x, N), @sext(x, N), @trunc(x, N),
    // @signed(x), @unsigned(x).
    Zext,
    Sext,
    Trunc,
    Signed,
    Unsigned,
    // Bit plumbing: @concat(a, b, ..) high-to-low, @rep(x, n), @zeroed().
    Concat,
    Rep,
    Zeroed,
    /// `@assert(cond)` / `@assert(cond, "message")` -- checked in simulation,
    /// absent from synthesis. `@fatal` is the same with `$fatal` instead of
    /// `$error`, for a condition there is no point continuing past.
    Assert,
    Fatal,
    // Channels.
    TrySend,
    TryRecieve,
    BlockingSend,
    BlockingRecieve
}

enum AnumResolution {
    Builtin(BuiltinOp),
    IntLiteral,
    Ref,
}

unsafe fn resolve_anum_span(anum_span: &AlphanumSpan) -> Result<AnumResolution, ()> {
    // let mut ptr = anum_span.byte_ptr;
    // let end_ptr = ptr.add(anum_span.len as _);
    let str = core::str::from_raw_parts(anum_span.byte_ptr, anum_span.len as _);
    let iden_is_builtin = str.as_bytes()[0] == b'@';
    if iden_is_builtin {
        match &str[1..] {
            "map" => return Ok(AnumResolution::Builtin(BuiltinOp::Simd)),
            "try_send" => return Ok(AnumResolution::Builtin(BuiltinOp::TrySend)),
            "try_rcv" => return Ok(AnumResolution::Builtin(BuiltinOp::TryRecieve)),
            "send" => return Ok(AnumResolution::Builtin(BuiltinOp::BlockingSend)),
            "rcv" => return Ok(AnumResolution::Builtin(BuiltinOp::BlockingRecieve)),
            // Width and signedness casts. The type system requires operands to
            // match exactly, so these are how you say what you meant instead
            // of letting the compiler guess.
            "zext" => return Ok(AnumResolution::Builtin(BuiltinOp::Zext)),
            "sext" => return Ok(AnumResolution::Builtin(BuiltinOp::Sext)),
            "trunc" => return Ok(AnumResolution::Builtin(BuiltinOp::Trunc)),
            "signed" => return Ok(AnumResolution::Builtin(BuiltinOp::Signed)),
            "unsigned" => return Ok(AnumResolution::Builtin(BuiltinOp::Unsigned)),
            // Bit plumbing.
            "concat" => return Ok(AnumResolution::Builtin(BuiltinOp::Concat)),
            "rep" => return Ok(AnumResolution::Builtin(BuiltinOp::Rep)),
            "zeroed" => return Ok(AnumResolution::Builtin(BuiltinOp::Zeroed)),
            "unreachable" => return Ok(AnumResolution::Builtin(BuiltinOp::Unreachable)),
            "cast" => return Ok(AnumResolution::Builtin(BuiltinOp::Cast)),
            "assert" => return Ok(AnumResolution::Builtin(BuiltinOp::Assert)),
            "fatal" => return Ok(AnumResolution::Builtin(BuiltinOp::Fatal)),
            _ => return Err(()), // we dont know this one
        }
    }
    let (ats_past_head, only_nums) = str.as_bytes().iter().fold((false, true), |acc, el| {
        (
            (*el == b'@') || acc.0,
            (*el as char).is_numeric() && acc.1,
        )
    });
    if ats_past_head {
        return Err(());
    }
    if only_nums {
        return Ok(AnumResolution::IntLiteral);
    }
    //
    Ok(AnumResolution::Ref)
}

unsafe fn resolve_stmt(char_ptr: *const u8, stmt: &InnerStmt) -> Result<PrecResInnerStmt, ()> {
    // this flattener is horrid! do better?
    fn flatten(expr: &RawExpr) -> &RawExpr {
        let mut expr = expr;
        // Peel `((x))` down to `x`: an infix node holding one subexpression
        // and no operator is parentheses that survived precedence resolution.
        while let RawExpr::InfixExpr { pieces } = expr {
            match &pieces[..] {
                [InfixExprComponent::Subexpr(inner)] => expr = inner,
                _ => break,
            }
        }
        expr
    }
    fn is_assign_smtm(expr: &InfixExprComponent) -> bool {
        matches!(expr, InfixExprComponent::Basic(BasicInfixOp::Assign(_)))
    }
    fn get_assign_stmt(expr: &InfixExprComponent) -> AssignStmtKind {
        match expr {
            InfixExprComponent::Basic(BasicInfixOp::Assign(ask)) => *ask,
            _ => unreachable!(),
        }
    }
    unsafe fn try_mk_assign_stmt(
        char_ptr: *const u8,
        pieces: &[InfixExprComponent],
    ) -> Result<AssignStmt, ()> {
        match pieces {
            [InfixExprComponent::Subexpr(lhs), op, tail @ ..] if is_assign_smtm(op) => {
                let left = resolve_precedence(char_ptr, lhs)?;
                let right = resolve_many(char_ptr, tail)?;
                let kind = get_assign_stmt(op);
                Ok(AssignStmt {
                    lvalue: left,
                    rvalue: right,
                    kind,
                })
            }
            _ => Err(()),
        }
    }
    unsafe fn wrap_in_block_if_standalone(
        char_ptr: *const u8,
        expr: &RawExpr,
    ) -> Result<PrecResExpr, ()> {
        match flatten(expr) {
            RawExpr::InfixExpr { pieces } => {
                let stmt = try_mk_assign_stmt(char_ptr, pieces)?;
                let rs = PrecResInnerStmt::AssignStmt(stmt);
                Ok(PrecResExpr::StmtBlock(StmtBlock {
                    components: vec![rs],
                }))
            }
            expr => resolve_precedence(char_ptr, expr),
        }
    }
    match stmt {
        InnerStmt::VarDecl(var_decl_stmt) => {
            let aval = if let Some(aval) = &var_decl_stmt.assign_val {
                Some(resolve_precedence(char_ptr, aval)?)
            } else {
                None
            };
            let tyval = if let Some(ty_expr) = &var_decl_stmt.ty_expr {
                Some(resolve_type(char_ptr, ty_expr)?)
            } else {
                None
            };
            Ok(PrecResInnerStmt::VarDecl(VarDeclStmt {
                is_mutable: var_decl_stmt.is_mutable,
                name: var_decl_stmt.name,
                rest: var_decl_stmt.rest.clone(),
                ty_expr: tyval,
                assign_val: aval,
            }))
        }
        InnerStmt::MatchStmt(match_stmt) => {
            let mut scruts = Vec::new();
            for item in &match_stmt.scrutinees {
                let item = resolve_precedence(char_ptr, item)?;
                scruts.push(item)
            }
            let mut cases = Vec::new();
            for item in &match_stmt.cases {
                let rhs = wrap_in_block_if_standalone(char_ptr, &item.rhs)?;
                cases.push(MatchArm {
                    binding_patterns: item.binding_patterns.clone(),
                    rhs,
                })
            }
            Ok(PrecResInnerStmt::MatchStmt(MatchStmt {
                scrutinees: scruts,
                cases,
            }))
        }
        InnerStmt::ExprStmt(raw_expr) => {
            // only valid as call or combined assign
            let flatten_expr = flatten(raw_expr);
            if let RawExpr::Call { base, args } = flatten_expr {
                let base = resolve_precedence(char_ptr, base)?;
                let mut argsp = Vec::new();
                for arg in args {
                    let arg = resolve_precedence(char_ptr, arg)?;
                    argsp.push(arg)
                }
                return Ok(PrecResInnerStmt::CallStmt(CallStmt { base, args: argsp }));
            }
            if let RawExpr::InfixExpr { pieces } = flatten_expr
                && let Ok(stmt) = try_mk_assign_stmt(char_ptr, pieces) {
                    return Ok(PrecResInnerStmt::AssignStmt(stmt));
                }
            let expr = resolve_precedence(char_ptr, flatten_expr)?;
            Ok(PrecResInnerStmt::TailVal(expr))
        }
        InnerStmt::IfThenElse(ite) => {
            // ite can have inline arms
            let cond = resolve_precedence(char_ptr, &ite.condition)?;

            let then_arm = &ite.then_case;
            let then_arm = wrap_in_block_if_standalone(char_ptr, then_arm)?;

            let else_arm = if let Some(expr) = &ite.else_case {
                Some(wrap_in_block_if_standalone(char_ptr, expr)?)
            } else {
                None
            };
            Ok(PrecResInnerStmt::IfThenElse(ITEStmt {
                condition: cond,
                then_case: then_arm,
                else_case: else_arm,
            }))
        }
        InnerStmt::Loop(loop_stmt) => {
            let lstmt = wrap_in_block_if_standalone(char_ptr, &loop_stmt.repeat_expr)?;
            Ok(PrecResInnerStmt::Loop(LoopStmt { repeat_expr: lstmt }))
        }
        InnerStmt::Break => Ok(PrecResInnerStmt::Break),
        InnerStmt::ForLoopStmt(stmt) => {
            let tar = resolve_precedence(char_ptr, &stmt.target)?;
            let body = resolve_precedence(char_ptr, &stmt.body)?;
            Ok(PrecResInnerStmt::ForLoop(Box::new(ForLoopStmt {
                binding: stmt.binding,
                target: tar,
                body,
            })))
        }
        InnerStmt::ReturnStmt(expr) => {
            let expr = match expr {
                Some(expr) => Some(resolve_precedence(char_ptr, expr)?),
                None => None,
            };
            Ok(PrecResInnerStmt::ReturnStmt(expr))
        }
    }
}

pub fn anumspan_to_str<'a>(span:&AlphanumSpan) -> &'a str {
    unsafe { core::str::from_raw_parts(span.byte_ptr, span.len as _) }
}

pub fn int_literal_to_int(anum_span: &AlphanumSpan) -> Result<usize, <usize as core::str::FromStr>::Err> {
    let str = anumspan_to_str(anum_span);
    str::parse::<usize>(str)
}

// resolve precedence and locally flatten the tree
/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence(char_ptr: *const u8, expr: &RawExpr) -> Result<PrecResExpr, ()> {
    match expr {
        // `if c then a else b` desugars to a three-operand select, so the
        // lowering has one mux path rather than two.
        RawExpr::Ternary { cond, then_e, else_e } => {
            let c = resolve_precedence(char_ptr, cond)?;
            let t = resolve_precedence(char_ptr, then_e)?;
            let e = resolve_precedence(char_ptr, else_e)?;
            Ok(PrecResExpr::Call {
                base: Box::new(PrecResExpr::Builtin(BuiltinOp::Select)),
                args: vec![c, t, e],
            })
        }

        RawExpr::StmtBlock(stmt_block) => {
            let mut checked_components = Vec::new();
            let components = stmt_block
                .components
                .iter()
                .map(|x| resolve_stmt(char_ptr, x));
            for component in components {
                checked_components.push(component?)
            }
            Ok(PrecResExpr::StmtBlock(StmtBlock {
                components: checked_components,
            }))
        }
        RawExpr::InfixExpr { pieces } => match &pieces[..] {
            [InfixExprComponent::Subexpr(subexpr)] => resolve_precedence(char_ptr, subexpr),
            pieces => resolve_many(char_ptr, pieces),
        },
        RawExpr::Call { base, args } => {
            let base = resolve_precedence(char_ptr, base)?;
            let mut proced = Vec::new();
            for item in args {
                let item = resolve_precedence(char_ptr, item)?;
                proced.push(item)
            }
            Ok(PrecResExpr::Call {
                base: Box::new(base),
                args: proced,
            })
        }
        RawExpr::NumLiteral(num) => {
            let lit = match num {
                RawNum::Int { width, value, .. } => Literal::IntLiteral {
                    value: *value,
                    width: *width,
                },
                RawNum::Float { whole, frac, .. } => Literal::FloatLiteral {
                    whole: *whole,
                    frac: *frac,
                },
            };
            Ok(PrecResExpr::Literal(lit))
        }
        RawExpr::Unary { op, operand } => {
            let operand = resolve_precedence(char_ptr, operand)?;
            let builtin = match op {
                UnaryOp::Neg => BuiltinOp::Neg,
                UnaryOp::BitNot => BuiltinOp::BitInvert,
                UnaryOp::LogNot => BuiltinOp::LogNot,
            };
            Ok(PrecResExpr::Call {
                base: Box::new(PrecResExpr::Builtin(builtin)),
                args: vec![operand],
            })
        }
        RawExpr::AnumSpan(anum_span) => match resolve_anum_span(anum_span)? {
            AnumResolution::Builtin(bio) => Ok(PrecResExpr::Builtin(bio)),
            AnumResolution::IntLiteral => {
                // Reachable only for digit runs that the numeric lexer did not
                // claim -- notably a tuple/field index after a dot.
                let outcome = int_literal_to_int(anum_span);
                match outcome {
                    Ok(int) => {
                        Ok(PrecResExpr::Literal(Literal::IntLiteral {
                            value: int as u128,
                            width: None,
                        }))
                    },
                    Err(_) => {
                        Err(())
                    },
                }
            }
            AnumResolution::Ref => Ok(PrecResExpr::Ref(*anum_span)),
        },
        RawExpr::MemberAccess { base, field_name } => {
            // The `<int>.<int>` reassembly that used to live here is gone:
            // try_parse_number recognises floats directly, so `0.717` never
            // reaches this point as a member access.
            let base = resolve_precedence(char_ptr, base)?;
            Ok(PrecResExpr::FieldAccess {
                base: Box::new(base),
                field_name: *field_name,
            })
        }
        RawExpr::SubscriptAccess(val) => {
            let base = resolve_precedence(char_ptr, &val.base)?;
            let index = resolve_precedence(char_ptr, &val.index)?;
            Ok(PrecResExpr::SubscriptAccess(Box::new(SubscriptAccess {
                base,
                index,
            })))
        }
        RawExpr::Splice(raw_exprs) => {
            let mut proced = Vec::new();
            for item in raw_exprs {
                let item = resolve_precedence(char_ptr, item)?;
                proced.push(item)
            }
            Ok(PrecResExpr::Splice(proced))
        }
        RawExpr::Postfix { base, op } => {
            let base = resolve_precedence(char_ptr, base)?;
            let op = match op {
                PostfixOp::Tilda => BuiltinOp::BitInvert,
            };
            Ok(PrecResExpr::Call {
                base: Box::new(PrecResExpr::Builtin(op)),
                args: vec![base],
            })
        }
        RawExpr::StrLiteral(str) => {
            Ok(PrecResExpr::Literal(Literal::StrLiteral(str.clone())))
        }
        RawExpr::Span(span) => {
            let left = resolve_precedence(char_ptr, &span.left)?;
            let right = resolve_precedence(char_ptr, &span.right)?;
            Ok(PrecResExpr::SubscriptAccess(Box::new(SubscriptAccess {
                base: left,
                index: right,
            })))
        }
    }
}

unsafe fn resolve_many(
    char_ptr: *const u8,
    pieces: &[InfixExprComponent],
) -> Result<PrecResExpr, ()> {
    // any assign ops are invalid here

    if let [InfixExprComponent::Subexpr(expr)] = pieces { return resolve_precedence(char_ptr, expr) }

    // Precedence groups, LOOSEST FIRST.
    //
    // The group that matches first becomes the OUTERMOST node, so this list
    // runs from loosest to tightest binding. The original had `* / %` before
    // `+ -`, which made `a + b * c` resolve to `(a + b) * c`.
    //
    // Within a group the split is at the RIGHTMOST operator for a
    // left-associative group, so `a - b - c` becomes `(a - b) - c`. The
    // original always split at the leftmost, making every operator
    // right-associative -- wrong for `-`, `/`, `%`, `<<` and `>>`.
    const LEFT: bool = false;
    const RIGHT: bool = true;

    #[rustfmt::skip]
    const PREC_GROUPS: &[(&[BasicInfixOp], bool)] = &[
        (&[BasicInfixOp::DotDot],                                          LEFT),
        (&[BasicInfixOp::VBarVBar],                                        LEFT),
        (&[BasicInfixOp::AmpAmp],                                          LEFT),
        (&[BasicInfixOp::VBar],                                            LEFT),
        (&[BasicInfixOp::Caret],                                           LEFT),
        (&[BasicInfixOp::Ampersand],                                       LEFT),
        (&[BasicInfixOp::EqEq, BasicInfixOp::NotEq],                       LEFT),
        (&[BasicInfixOp::Lt, BasicInfixOp::Gt,
           BasicInfixOp::LtEq, BasicInfixOp::GtEq],                        LEFT),
        (&[BasicInfixOp::DoubleBraketLeft,
           BasicInfixOp::DoubleBraketRight],                               LEFT),
        (&[BasicInfixOp::Plus, BasicInfixOp::Minus],                       LEFT),
        (&[BasicInfixOp::Star, BasicInfixOp::Slash,
           BasicInfixOp::Percent],                                         LEFT),
        // `**` binds tightest and is right-associative: 2**3**2 is 2**(3**2).
        (&[BasicInfixOp::StarStar],                                        RIGHT),
    ];

    /// Index of the operator this group should split at, searching the whole
    /// group at once. The original looped operator-by-operator, so within
    /// `[*, /, %]` it found every `*` before considering any `/` -- making
    /// `a * b / c` resolve as `a * (b / c)`.
    fn find_split(
        group: &[BasicInfixOp],
        pieces: &[InfixExprComponent],
        right_assoc: bool,
    ) -> Option<(usize, BasicInfixOp)> {
        let mut found: Option<(usize, BasicInfixOp)> = None;
        for (ix, piece) in pieces.iter().enumerate() {
            let op = match piece {
                InfixExprComponent::Basic(op) => *op,
                InfixExprComponent::Subexpr(_) => continue,
            };
            if !group.contains(&op) {
                continue;
            }
            if right_assoc {
                // Leftmost wins, so the rightmost operator ends up deepest.
                return Some((ix, op));
            }
            found = Some((ix, op));
        }
        found
    }

    fn builtin_for(op: BasicInfixOp) -> Option<BuiltinOp> {
        let b = match op {
            BasicInfixOp::Plus => BuiltinOp::Add,
            BasicInfixOp::Minus => BuiltinOp::Sub,
            BasicInfixOp::Star => BuiltinOp::Mul,
            BasicInfixOp::StarStar => BuiltinOp::Pow,
            BasicInfixOp::Slash => BuiltinOp::Div,
            BasicInfixOp::Percent => BuiltinOp::Mod,
            BasicInfixOp::DoubleBraketLeft => BuiltinOp::Shl,
            BasicInfixOp::DoubleBraketRight => BuiltinOp::Shr,
            BasicInfixOp::Ampersand => BuiltinOp::And,
            BasicInfixOp::VBar => BuiltinOp::Or,
            BasicInfixOp::Caret => BuiltinOp::Xor,
            BasicInfixOp::AmpAmp => BuiltinOp::LogAnd,
            BasicInfixOp::VBarVBar => BuiltinOp::LogOr,
            BasicInfixOp::EqEq => BuiltinOp::Eq,
            BasicInfixOp::NotEq => BuiltinOp::Ne,
            BasicInfixOp::Lt => BuiltinOp::Lt,
            BasicInfixOp::Gt => BuiltinOp::Gt,
            BasicInfixOp::LtEq => BuiltinOp::Le,
            BasicInfixOp::GtEq => BuiltinOp::Ge,
            // Handled by the caller: a range is not a call.
            BasicInfixOp::DotDot => return None,
            // Assignment is a statement, never an expression.
            BasicInfixOp::Assign(_) => return None,
        };
        Some(b)
    }

    for (group, right_assoc) in PREC_GROUPS {
        let (ix, op) = match find_split(group, pieces, *right_assoc) {
            Some(hit) => hit,
            None => continue,
        };
        let lhs_pieces = &pieces[..ix];
        let rhs_pieces = &pieces[ix + 1..];
        if lhs_pieces.is_empty() || rhs_pieces.is_empty() {
            return Err(());
        }
        let lhs = resolve_many(char_ptr, lhs_pieces)?;
        let rhs = resolve_many(char_ptr, rhs_pieces)?;

        if op == BasicInfixOp::DotDot {
            return Ok(PrecResExpr::Span(Box::new(Span { left: lhs, right: rhs })));
        }
        let bin = match builtin_for(op) {
            Some(b) => b,
            None => return Err(()),
        };
        return Ok(PrecResExpr::Call {
            base: Box::new(PrecResExpr::Builtin(bin)),
            args: vec![lhs, rhs],
        });
    }

    Err(())
}

unsafe fn resolve_type(
    char_ptr: *const u8,
    raw_type_expr: &RawTypeExpr,
) -> Result<PrecTypeExpr, ()> {
    match raw_type_expr {
        RawTypeExpr::Ident(alphanum_span) => {
            Ok(PrecTypeExpr::Ident(*alphanum_span))
        },
        RawTypeExpr::Array(item_type, count) => {
            let resolved_ty = resolve_type(char_ptr, item_type)?;
            let resolved_count = resolve_precedence(char_ptr, count)?;
            Ok(PrecTypeExpr::Array(Box::new(resolved_ty), resolved_count))
        },
        RawTypeExpr::MemArray { elem, len, kind } => {
            let resolved_elem = resolve_type(char_ptr, elem)?;
            let resolved_len = resolve_precedence(char_ptr, len)?;
            Ok(PrecTypeExpr::MemArray {
                elem: Box::new(resolved_elem),
                len: resolved_len,
                kind: *kind,
            })
        },
    }
}

unsafe fn resolve_arg_tuple(
    char_ptr: *const u8,
    arg_tuple: &RawArgTuple,
) -> Result<PrecArgDefTuple, ()> {
    let entries = &arg_tuple.entries;
    let mut result = Vec::new();
    for item in entries {
        let ty = &item.type_expr;
        let k = resolve_type(char_ptr, ty)?;
        let default = match &item.default {
            Some(e) => Some(resolve_precedence(char_ptr, e)?),
            None => None,
        };
        let x = PrecArgTupleEntry {
            arg_name: item.arg_name,
            qualifier: item.qualifier,
            type_expr: k,
            default,
        };
        result.push(x);
    }
    Ok(PrecArgDefTuple { entries: result })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence_for_process(
    char_ptr: *const u8,
    proc_decl: &RawProcessDecl,
) -> Result<ProcessDecl, ()> {
    let mut items = Vec::new();
    for item in &proc_decl.body {
        let x = resolve_stmt(char_ptr, item);
        items.push(x?)
    }
    let args = resolve_arg_tuple(char_ptr, &proc_decl.args)?;
    Ok(ProcessDecl {
        name: proc_decl.name,
        args,
        body: items,
    })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence_for_function(
    char_ptr: *const u8,
    proc_decl: &RawFunctionDecl,
) -> Result<FunctionDecl, ()> {
    let mut items = Vec::new();
    for x in proc_decl
        .body
        .iter()
        .map(|item| resolve_stmt(char_ptr, item))
    {
        items.push(x?)
    }
    let args = resolve_arg_tuple(char_ptr, &proc_decl.args)?;
    Ok(FunctionDecl {
        name: proc_decl.name,
        args,
        body: items,
    })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence_for_sequence(
    char_ptr: *const u8,
    proc_decl: &RawSequenceDecl,
) -> Result<SequenceDecl, ()> {
    let mut items = Vec::new();
    for item in &proc_decl.body {
        let item = match item {
            RawSeqInnerStmt::SegmentSeparator => {
                PrecSeqInnerStmt::SegmentSeparator
            },
            RawSeqInnerStmt::Stmt(inner_stmt) => {
                PrecSeqInnerStmt::Stmt(resolve_stmt(char_ptr, inner_stmt)?)
            },
        };
        items.push(item);
    }
    let args = resolve_arg_tuple(char_ptr, &proc_decl.args)?;
    Ok(SequenceDecl {
        name: proc_decl.name,
        args,
        body: items,
    })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
/// A graph body holds no expressions, so this only turns raw type
/// expressions into resolved ones.
pub unsafe fn resolve_precedence_for_graph(
    char_ptr: *const u8,
    graph_decl: &RawGraphDecl,
) -> Result<GraphDecl, ()> {
    let mut body = Vec::new();
    for item in &graph_decl.body {
        let item = match item {
            RawGraphStmt::Pipe(pipe) => GraphStmt::Pipe(GraphPipe {
                name: pipe.name,
                said: pipe.said,
                ty: resolve_type(char_ptr, &pipe.type_expr)?,
            }),
            RawGraphStmt::Instance(inst) => GraphStmt::Instance(GraphInstance {
                module: inst.module,
                args: inst.args.clone(),
            }),
        };
        body.push(item);
    }
    let args = resolve_arg_tuple(char_ptr, &graph_decl.args)?;
    Ok(GraphDecl { name: graph_decl.name, args, body })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence_for_struct(
    char_ptr: *const u8,
    struct_decl: &RawStructDecl,
) -> Result<StructDecl, ()> {
    let mut fields = Vec::new();
    for field in &struct_decl.fields {
        let x = resolve_type(char_ptr, &field.field_type)?;
        let x = StructField {
            name: field.name,
            field_type: x,
        };
        fields.push(x);
    }
    Ok(StructDecl { name: struct_decl.name, fields })
}

/// # Safety
///
/// `char_ptr` must be the base of the same buffer the raw AST was parsed
/// from: the spans in it are pointers into that buffer, and resolving one
/// dereferences them.
pub unsafe fn resolve_precedence_for_enum(
    char_ptr: *const u8,
    enum_decl: &RawEnumDecl,
) -> Result<EnumDecl, ()> {
    let tag_type = match &enum_decl.tag_type {
        Some(t) => Some(resolve_type(char_ptr, t)?),
        None => None,
    };
    let mut variants = Vec::new();
    for field in &enum_decl.fields {
        let discriminant = match &field.discriminant {
            Some(e) => Some(resolve_precedence(char_ptr, e)?),
            None => None,
        };
        let payload = match &field.payload {
            Some(t) => Some(resolve_type(char_ptr, t)?),
            None => None,
        };
        variants.push(EnumVariant { name: field.name, discriminant, payload });
    }
    Ok(EnumDecl { name: enum_decl.name, tag_type, variants })
}

#[test]
fn t1() {
    use crate::lex::parse_top_level;
    let str = concat!(
        "process Name (arg1: Ty)\n",
        "   \n    \n",
        // "  for i in 1..0\n",
        // "    let _ = \n",
        // "      break\n",
        // "    return\n",
        // "  expr\n",
        "  let _ : [[i1;2];2] = expr\n"
    );
    let inp_str = str.as_bytes().as_ptr_range();
    let start_ptr = inp_str.start;
    let end_ptr = inp_str.end;
    let span = (end_ptr as usize) - (start_ptr as usize);
    let smth = match unsafe { parse_top_level(start_ptr, span as u32) } {
        Ok(val) => val,
        Err(_) => panic!("should parse"),
    };

    let item = match &smth[0] {
        crate::lex::TopLevelDecl::ProcessStmt(decl) => decl,
        _ => panic!("Expected ProcessStmt"),
    };
    let item = unsafe { resolve_precedence_for_process(start_ptr, item) }
        .expect("should resolve");

    assert_eq!(anumspan_to_str(&item.name), "Name");
    assert_eq!(item.body.len(), 1);
    // A nested array type survives resolution with both lengths intact.
    let decl = match &item.body[0] {
        PrecResInnerStmt::VarDecl(v) => v,
        other => panic!("expected a var decl, got {:?}", other),
    };
    match decl.ty_expr.as_ref().expect("has a type") {
        PrecTypeExpr::Array(inner, outer_len) => {
            assert!(matches!(
                const_len(outer_len),
                Some(2)
            ));
            match &**inner {
                PrecTypeExpr::Array(elem, inner_len) => {
                    assert!(matches!(const_len(inner_len), Some(2)));
                    assert!(matches!(&**elem, PrecTypeExpr::Ident(_)));
                }
                other => panic!("expected an inner array, got {:?}", other),
            }
        }
        other => panic!("expected an array type, got {:?}", other),
    }
}

#[cfg(test)]
fn const_len(expr: &PrecResExpr) -> Option<u128> {
    match expr {
        PrecResExpr::Literal(Literal::IntLiteral { value, .. }) => Some(*value),
        _ => None,
    }
}


#[test]
fn t2() {
    // Was: `0.717.1` and `@try_pop(smth).1`. It asserted nothing and failed --
    // `@try_pop` is not a builtin, so resolve_anum_span returned Err and the
    // unwrap below panicked. Rewritten to check what it was reaching for:
    // a float literal and a numeric field index, both of which now go through
    // the numeric lexer rather than the member-access reassembly hack.
    use crate::lex::parse_top_level;
    let str = concat!(
        "sequence Name (smth: stream in Ty)\n",
        "   \n    \n",
        "  let a = 0.717\n",
        "  let b = @try_rcv(smth).1\n",
    );
    let inp_str = str.as_bytes().as_ptr_range();
    let start_ptr = inp_str.start;
    let end_ptr = inp_str.end;
    let span = (end_ptr as usize) - (start_ptr as usize);
    let decls = unsafe { parse_top_level(start_ptr, span as u32) }
        .unwrap_or_else(|_| panic!("should parse"));

    let item = match &decls[0] {
        crate::lex::TopLevelDecl::SequenceDecl(decl) => decl,
        other => panic!("expected a sequence, got {:?}", other),
    };
    let item = unsafe { resolve_precedence_for_sequence(start_ptr, item) }
        .expect("should resolve");

    assert_eq!(item.body.len(), 2);

    let float = match &item.body[0] {
        PrecSeqInnerStmt::Stmt(PrecResInnerStmt::VarDecl(v)) => v.assign_val.clone(),
        other => panic!("expected a var decl, got {:?}", other),
    };
    match float {
        Some(PrecResExpr::Literal(Literal::FloatLiteral { whole, frac })) => {
            assert_eq!((whole, frac), (0, 717));
        }
        other => panic!("expected a float literal, got {:?}", other),
    }

    // `.1` after a call is a field access, not part of a float.
    let field = match &item.body[1] {
        PrecSeqInnerStmt::Stmt(PrecResInnerStmt::VarDecl(v)) => v.assign_val.clone(),
        other => panic!("expected a var decl, got {:?}", other),
    };
    match field {
        Some(PrecResExpr::FieldAccess { base, field_name }) => {
            assert_eq!(anumspan_to_str(&field_name), "1");
            assert!(matches!(*base, PrecResExpr::Call { .. }));
        }
        other => panic!("expected a field access, got {:?}", other),
    }
}

// ---- precedence ----------------------------------------------------------
//
// The bug these exist for: the group table was ordered `* / %` before `+ -`,
// and the FIRST group matched becomes the OUTERMOST node -- so `a + b * c`
// resolved to `(a + b) * c`. Nothing caught it because no test built an
// arithmetic expression.

#[cfg(test)]
fn resolve_expr_in(src_body: &str) -> PrecResExpr {
    use crate::lex::parse_top_level;
    // Leaked deliberately. `AlphanumSpan` is a bare `*const u8` with no
    // lifetime, so the returned expression points into this buffer -- letting
    // a local `String` drop here is a use-after-free that reads correctly
    // often enough to produce an intermittently passing test. Leaking gives
    // the pointers a genuinely 'static target. This is the clearest argument
    // for interning identifiers; see the note in src/diag.rs.
    let src: &'static str = Box::leak(
        format!("fun f (a: i32, b: i32, c: i32)\n  let out = {}\n", src_body)
            .into_boxed_str(),
    );
    let range = src.as_bytes().as_ptr_range();
    let len = (range.end as usize) - (range.start as usize);
    let decls = unsafe { parse_top_level(range.start, len as u32) }
        .unwrap_or_else(|_| panic!("`{}` should parse", src_body));
    let f = match &decls[0] {
        crate::lex::TopLevelDecl::FunctionStmt(d) => d,
        other => panic!("expected a fun, got {:?}", other),
    };
    let f = unsafe { resolve_precedence_for_function(range.start, f) }
        .unwrap_or_else(|_| panic!("`{}` should resolve", src_body));
    match &f.body[0] {
        PrecResInnerStmt::VarDecl(v) => v.assign_val.clone().expect("has an initialiser"),
        other => panic!("expected a var decl, got {:?}", other),
    }
}

/// Renders a resolved expression as a fully parenthesised prefix form, so a
/// test can state the expected tree shape in one readable line.
#[cfg(test)]
fn shape(expr: &PrecResExpr) -> String {
    match expr {
        PrecResExpr::Ref(s) => anumspan_to_str(s).to_string(),
        PrecResExpr::Literal(Literal::IntLiteral { value, width: None }) => format!("{}", value),
        PrecResExpr::Literal(Literal::IntLiteral { value, width: Some(w) }) => {
            format!("{}'{}", w, value)
        }
        PrecResExpr::Literal(lit) => format!("{:?}", lit),
        PrecResExpr::Builtin(op) => format!("{:?}", op),
        PrecResExpr::Call { base, args } => {
            let head = match &**base {
                PrecResExpr::Builtin(op) => format!("{:?}", op),
                other => shape(other),
            };
            let rendered: Vec<String> = args.iter().map(shape).collect();
            format!("({} {})", head, rendered.join(" "))
        }
        PrecResExpr::Span(s) => format!("(Range {} {})", shape(&s.left), shape(&s.right)),
        PrecResExpr::FieldAccess { base, field_name } => {
            format!("(. {} {})", shape(base), anumspan_to_str(field_name))
        }
        PrecResExpr::SubscriptAccess(s) => {
            format!("([] {} {})", shape(&s.base), shape(&s.index))
        }
        other => format!("{:?}", other),
    }
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    assert_eq!(shape(&resolve_expr_in("a + b * c")), "(Add a (Mul b c))");
    assert_eq!(shape(&resolve_expr_in("a * b + c")), "(Add (Mul a b) c)");
}

#[test]
fn arithmetic_is_left_associative() {
    assert_eq!(shape(&resolve_expr_in("a - b - c")), "(Sub (Sub a b) c)");
    assert_eq!(shape(&resolve_expr_in("a / b / c")), "(Div (Div a b) c)");
    // Mixed operators of equal precedence split at the rightmost, so this is
    // (a * b) / c and not a * (b / c).
    assert_eq!(shape(&resolve_expr_in("a * b / c")), "(Div (Mul a b) c)");
}

#[test]
fn power_is_right_associative_and_binds_tightest() {
    assert_eq!(shape(&resolve_expr_in("a ** b ** c")), "(Pow a (Pow b c))");
    assert_eq!(shape(&resolve_expr_in("a * b ** c")), "(Mul a (Pow b c))");
}

#[test]
fn full_precedence_ladder() {
    // Loosest to tightest: || && | ^ & ==/!= relational shift +- */% **
    assert_eq!(
        shape(&resolve_expr_in("a | b & c")),
        "(Or a (And b c))"
    );
    assert_eq!(
        shape(&resolve_expr_in("a == b + c")),
        "(Eq a (Add b c))"
    );
    assert_eq!(
        shape(&resolve_expr_in("a < b << c")),
        "(Lt a (Shl b c))"
    );
    assert_eq!(
        shape(&resolve_expr_in("a && b || c")),
        "(LogOr (LogAnd a b) c)"
    );
    assert_eq!(
        shape(&resolve_expr_in("a ^ b & c")),
        "(Xor a (And b c))"
    );
}

#[test]
fn parentheses_override_precedence() {
    assert_eq!(shape(&resolve_expr_in("(a + b) * c")), "(Mul (Add a b) c)");
}

#[test]
fn comparison_operators_all_resolve() {
    assert_eq!(shape(&resolve_expr_in("a != b")), "(Ne a b)");
    assert_eq!(shape(&resolve_expr_in("a <= b")), "(Le a b)");
    assert_eq!(shape(&resolve_expr_in("a >= b")), "(Ge a b)");
    assert_eq!(shape(&resolve_expr_in("a > b")), "(Gt a b)");
}

#[test]
fn unary_operators_resolve() {
    assert_eq!(shape(&resolve_expr_in("-a")), "(Neg a)");
    assert_eq!(shape(&resolve_expr_in("~a")), "(BitInvert a)");
    assert_eq!(shape(&resolve_expr_in("!a")), "(LogNot a)");
    // Unary binds tighter than any infix operator.
    assert_eq!(shape(&resolve_expr_in("-a + b")), "(Add (Neg a) b)");
    // `!=` must not be read as `!` applied to `= b`.
    assert_eq!(shape(&resolve_expr_in("a != b")), "(Ne a b)");
}

#[test]
fn sized_and_radix_literals() {
    assert_eq!(shape(&resolve_expr_in("8'hFF")), "8'255");
    assert_eq!(shape(&resolve_expr_in("4'b1010")), "4'10");
    assert_eq!(shape(&resolve_expr_in("16'd12")), "16'12");
    assert_eq!(shape(&resolve_expr_in("1'b0")), "1'0");
    assert_eq!(shape(&resolve_expr_in("0xDEAD_BEEF")), "3735928559");
    assert_eq!(shape(&resolve_expr_in("0b1010")), "10");
    assert_eq!(shape(&resolve_expr_in("0o17")), "15");
    assert_eq!(shape(&resolve_expr_in("1_000_000")), "1000000");
    // desc.md:163 spells a clock frequency this way.
    assert_eq!(shape(&resolve_expr_in("12*10**6")), "(Mul 12 (Pow 10 6))");
}

#[test]
fn ranges_are_looser_than_arithmetic() {
    assert_eq!(shape(&resolve_expr_in("a + 1 .. b - 1")), "(Range (Add a 1) (Sub b 1))");
}

#[test]
fn t3() {
    use crate::lex::parse_top_level;
    let str = concat!(
        "struct IntPair\n",
        "  fst: i1\n",
        "  snd: i1\n",
    );
    let inp_str = str.as_bytes().as_ptr_range();
    let start_ptr = inp_str.start;
    let end_ptr = inp_str.end;
    let span = (end_ptr as usize) - (start_ptr as usize);
    let smth = match unsafe { parse_top_level(start_ptr, span as u32) } {
        Ok(val) => val,
        Err(_) => panic!("should parse"),
    };
    // println!("{:#?}", smth);

    let item = match &smth[0] {
        crate::lex::TopLevelDecl::StructDecl(decl) => decl,
        _ => panic!("Expected ProcessStmt"),
    };
    let item = unsafe { resolve_precedence_for_struct(start_ptr, item) }
        .expect("should resolve");

    assert_eq!(anumspan_to_str(&item.name), "IntPair");
    let fields: Vec<&str> = item
        .fields
        .iter()
        .map(|f| anumspan_to_str(&f.name))
        .collect();
    assert_eq!(fields, ["fst", "snd"]);
}