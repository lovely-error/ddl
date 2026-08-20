#[allow(nonstandard_style)]
mod Letters {
    pub const A: u8 = b'A';
    pub const Z: u8 = b'Z';
    pub const a: u8 = b'a';
    pub const z: u8 = b'z';
    pub const Underscore: u8 = b'_';
    pub const Whitespace: u8 = b' ';
    pub const NewLine: u8 = b'\n';
    pub const CarriageReturn: u8 = b'\r';
    pub const Tab: u8 = b'\t';
    pub const Dash: u8 = b'-';
    pub const ZERO: u8 = 48;
    pub const NINE: u8 = 57;
    pub const AT_SIGN: u8 = b'@';
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasicInfixOp {
    Plus,              // +
    Minus,             // -
    Star,              // *
    StarStar,          // **
    Slash,             // /
    DoubleBraketLeft,  // <<
    DoubleBraketRight, // >>
    Percent,           // %
    Ampersand,         // &
    VBar,              // |
    Caret,             // ^
    AmpAmp,            // &&
    VBarVBar,          // ||
    EqEq,              // ==
    NotEq,             // !=
    Lt,                // <
    Gt,                // >
    LtEq,              // <=
    GtEq,              // >=
    DotDot,            // ..
    Assign(AssignStmtKind),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignStmtKind {
    PlainAssign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    ShlAssign,
    ShrAssign,
    AndAssign,
    OrAssign,
    XorAssign,
    InvertAssign,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    /// `-x`, two's complement negation.
    Neg,
    /// `~x`, bitwise inversion. Also spelled `x.~` as a postfix.
    BitNot,
    /// `!x`, logical negation, always yields `i1`.
    LogNot,
}

#[derive(Debug, Clone)]
pub enum InfixExprComponent {
    Basic(BasicInfixOp),
    Subexpr(RawExpr),
}
type InfixExpr = Vec<InfixExprComponent>;

#[derive(Debug, Clone, Copy)]
pub enum PostfixOp {
    Tilda,
}

/// A numeric literal as written.
///
/// Width is carried from the source because an HDL cannot infer it: `8'h00`
/// and `16'h0000` are different hardware. An unsized literal has `width: None`
/// and takes its width from context.
#[derive(Debug, Clone, Copy)]
pub enum RawNum {
    Int {
        span: AlphanumSpan,
        width: Option<u32>,
        value: u128,
    },
    Float {
        span: AlphanumSpan,
        whole: u64,
        frac: u64,
    },
}

impl RawNum {
    pub fn span(&self) -> AlphanumSpan {
        match self {
            RawNum::Int { span, .. } => *span,
            RawNum::Float { span, .. } => *span,
        }
    }
}

#[derive(Debug,Clone)]
pub enum RawExpr {
    AnumSpan(AlphanumSpan), // ascii letters
    NumLiteral(RawNum),
    // <op> expr
    Unary {
        op: UnaryOp,
        operand: IndirectRawExpr,
    },
    // .member_name
    MemberAccess {
        base: Box<RawExpr>,
        field_name: AlphanumSpan,
    },
    // expr <op>
    Postfix {
        base: IndirectRawExpr,
        op: PostfixOp,
    },
    // expr (args,*)
    Call {
        base: IndirectRawExpr,
        args: Vec<RawExpr>,
    },
    // [expr]
    SubscriptAccess(Box<SubscriptAccess>), 
    // '{ [a..b], c[d..e], f }
    Splice(Vec<RawExpr>),
    // expr <op> expr
    InfixExpr {
        pieces: InfixExpr,
    },
    StmtBlock(StmtBlock),
    /// `if c then a else b` in expression position.
    ///
    /// The `else` is mandatory here: an expression must have a value on every
    /// path, and a missing one would be a latch rather than a default.
    Ternary {
        cond: Box<RawExpr>,
        then_e: Box<RawExpr>,
        else_e: Box<RawExpr>,
    },
    StrLiteral(StrLiteral),
    Span(Box<Span>), // expr .. expr
}
type IndirectRawExpr = Box<RawExpr>;

#[derive(Debug, Clone)]
pub struct SubscriptAccess {
    pub base: RawExpr,
    pub index: RawExpr,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub left: RawExpr,
    pub right: RawExpr,
}

#[derive(Debug, Clone)]
pub struct StmtBlock {
    pub components: Vec<InnerStmt>,
}

#[derive(Debug, Clone)]
pub enum InnerStmt {
    VarDecl(VarDeclStmt),
    MatchStmt(MatchStmt),
    ExprStmt(RawExpr),
    IfThenElse(ITEStmt),
    Loop(LoopStmt),
    Break,
    ForLoopStmt(ForLoopStmt),
    ReturnStmt(Option<RawExpr>),
}

#[derive(Debug, Clone)]
pub struct ForLoopStmt {
    pub binding: AlphanumSpan,
    pub target: RawExpr,
    pub body: RawExpr,
}

#[derive(Debug, Clone)]
pub struct LoopStmt {
    pub repeat_expr: RawExpr,
}

#[derive(Debug, Clone)]
pub struct ITEStmt {
    pub condition: RawExpr,
    pub then_case: RawExpr,
    pub else_case: Option<RawExpr>,
}

#[derive(Debug, Clone)]
pub struct VarDeclStmt {
    pub is_mutable: bool,
    pub name: AlphanumSpan,
    /// The remaining names of a tuple binding: `let (val, ok) = ...` puts
    /// `val` in `name` and `ok` here. Empty for an ordinary declaration.
    pub rest: Vec<AlphanumSpan>,
    pub ty_expr: Option<RawTypeExpr>,
    pub assign_val: Option<RawExpr>,
}

#[derive(Debug, Clone)]
pub struct MatchStmt {
    pub scrutinees: Vec<RawExpr>,
    pub cases: Vec<MatchArm>,
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub binding_patterns: Vec<BindingPattern>,
    pub rhs: RawExpr,
}

#[derive(Debug, Clone)]
pub enum BindingPattern {
    Alphanum(AlphanumSpan),
    EnumCase {
        base: AlphanumSpan,
        subbinding: Option<AlphanumSpan>,
    },
    /// `.A | .B | .C` -- alternatives for ONE scrutinee position.
    ///
    /// The enclosing `Vec<BindingPattern>` is still one entry per scrutinee;
    /// this nests inside a single entry so that shape does not change.
    AnyOf(Vec<BindingPattern>),
}

#[derive(Clone, Copy)]
pub struct AlphanumSpan {
    pub byte_ptr: *const u8,
    pub len: u32,
}

/// Prints the identifier, not the pointer.
///
/// Every AST dump goes through this, and a tree of raw addresses says nothing
/// about the source it came from. The text is borrowed from the SourceMap,
/// which outlives every AST that refers to it.
impl core::fmt::Debug for AlphanumSpan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = unsafe { core::str::from_raw_parts(self.byte_ptr, self.len as usize) };
        write!(f, "`{}`", text)
    }
}

impl core::fmt::Debug for StrSpan {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

#[derive(Debug)]
pub struct RawProcessDecl {
    pub name: AlphanumSpan,
    pub args: RawArgDefTuple,
    pub body: Vec<InnerStmt>,
}
#[derive(Debug)]
pub enum RawSeqInnerStmt {
    SegmentSeparator,
    Stmt(InnerStmt)
}

#[derive(Debug)]
pub struct RawSequenceDecl {
    pub name: AlphanumSpan,
    pub args: RawArgDefTuple,
    pub body: Vec<RawSeqInnerStmt>,
}

#[derive(Debug)]
pub struct RawFunctionDecl {
    pub name: AlphanumSpan,
    pub args: RawArgDefTuple,
    pub return_type: Option<RawTypeExpr>,
    pub body: Vec<InnerStmt>,
}

#[derive(Debug, Clone)]
pub struct RawArgTupleEntry {
    pub arg_name: AlphanumSpan,
    pub qualifier: ArgTypeQualifier,
    pub type_expr: RawTypeExpr,
    /// `= <expr>`. A process parameter that is not a pipe is a constant, and
    /// this is where its value comes from.
    pub default: Option<RawExpr>,
}
#[derive(Debug, Clone)]
pub struct RawArgDefTuple {
    pub entries: Vec<RawArgTupleEntry>,
}

#[derive(Debug, Clone)]
pub enum RawTypeExpr {
    Ident(AlphanumSpan), 
    Array(Box<RawTypeExpr>, RawExpr),
    /// `#[impl(lutram)] [T; n]` -- an array that asks for a particular backing
    /// store rather than being a packed vector. desc.md:92.
    MemArray { elem: Box<RawTypeExpr>, len: RawExpr, kind: AlphanumSpan },
}
#[derive(Debug, Clone, Copy)]
pub enum ArgTypeQualifier {
    Out,
    In,
    Inout,
    BufferIn,
    BufferOut,
    /// `stream in` / `stream out`, which the language no longer has.
    ///
    /// Still lexed, and refused during lowering with a message that says what
    /// to write instead -- the same treatment `#[impl(bkram)]` gets. Deleting
    /// it from the lexer instead would make `a: stream in i8` fail as an
    /// unrecognised type, which blames the wrong word.
    StreamIn,
    StreamOut,
}
#[derive(Debug, Clone)]
pub struct RawStructField {
    pub name: AlphanumSpan,
    pub field_type: RawTypeExpr
}
#[derive(Debug, Clone)]
pub struct RawStructDecl {
    pub name: AlphanumSpan,
    pub fields: Vec<RawStructField>
}

#[derive(Debug, Clone)]
pub struct RawEnumField {
    pub name: AlphanumSpan,
    /// `= <const>`. Absent means one more than the previous variant, so the
    /// common case of a dense enum needs no numbers at all.
    pub discriminant: Option<RawExpr>,
    /// `: T` payload, the tagged-union form of desc.md:78. Parsed so the
    /// syntax has a home, but rejected downstream -- a data-carrying variant
    /// needs a layout algorithm that does not exist yet.
    pub payload: Option<RawTypeExpr>,
}
#[derive(Debug, Clone)]
pub struct RawEnumDecl {
    pub name: AlphanumSpan,
    /// `enum Name: i2`. Absent means the width is inferred from the largest
    /// discriminant.
    pub tag_type: Option<RawTypeExpr>,
    pub fields: Vec<RawEnumField>
}

#[derive(Debug)]
pub enum TopLevelDecl {
    ProcessStmt(RawProcessDecl),
    FunctionStmt(RawFunctionDecl),
    SequenceDecl(RawSequenceDecl),
    StructDecl(RawStructDecl),
    EnumDecl(RawEnumDecl),
    GraphDecl(RawGraphDecl),
}

/// `graph Name (ports)` -- structural composition and nothing else.
///
/// A graph body holds no expressions, because a graph computes nothing. It
/// declares internal pipes and instantiates processes and sequences on them,
/// which is why it has its own tiny statement parser rather than reusing the
/// one that knows about arithmetic.
#[derive(Debug)]
pub struct RawGraphDecl {
    pub name: AlphanumSpan,
    pub args: RawArgDefTuple,
    pub body: Vec<RawGraphStmt>,
}

#[derive(Debug)]
pub enum RawGraphStmt {
    /// `let name: buffer T`
    Pipe(RawGraphPipe),
    /// `Name(a, b, c)`
    Instance(RawGraphInstance),
}

/// Which word a graph's `let` used to say what kind of pipe it declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeWord {
    Buffer,
    /// Recognised only so lowering can say the language has no streams.
    Stream,
    Missing,
}

#[derive(Debug)]
pub struct RawGraphPipe {
    pub name: AlphanumSpan,
    /// `stream` was written where `buffer` belongs, or neither word was.
    ///
    /// Parsed rather than rejected so that lowering can say which word is
    /// missing and point at the line -- the parser has no diagnostics, and
    /// failing here blames the whole `graph` declaration for a typo on one
    /// line of it.
    pub said: PipeWord,
    pub type_expr: RawTypeExpr,
}

#[derive(Debug)]
pub struct RawGraphInstance {
    /// The `process` or `sequence` being instantiated.
    pub module: AlphanumSpan,
    /// One name per pipe parameter, in declaration order. Each names either a
    /// port of the enclosing graph or a pipe declared in it.
    pub args: Vec<AlphanumSpan>,
}

fn deref<T>(ptr: *const T) -> T where T:Copy {
    unsafe { ptr.read() }
}

#[inline(never)]
fn strip_prefix_on_match(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    prefix: &str,
) -> (bool, *const u8) {
    let mut local_char_ptr = char_ptr;
    let ptr_span = prefix.as_bytes().as_ptr_range();
    let mut prefix_pivot_ptr = ptr_span.start;
    let prefix_end_ptr = ptr_span.end;
    loop {
        // Both end conditions are tested BEFORE either dereference. The
        // original tested them after, so probing at char_ptr == char_end_ptr
        // read one byte past the buffer.
        if prefix_pivot_ptr == prefix_end_ptr {
            return (true, local_char_ptr);
        }
        if local_char_ptr == char_end_ptr {
            return (false, char_ptr);
        }
        let chr1 = deref(local_char_ptr);
        let chr2 = deref(prefix_pivot_ptr);
        if chr1 != chr2 {
            return (false, char_ptr);
        }
        local_char_ptr = unsafe { local_char_ptr.byte_add(1) };
        prefix_pivot_ptr = unsafe { prefix_pivot_ptr.byte_add(1) };
    }
}

// Runs to just before the line terminator, leaving it for skip_trivia to
// consume. Stops at EOF, so an unterminated comment on the last line is fine.
fn skip_to_line_end(char_ptr: *const u8, char_end_ptr: *const u8) -> *const u8 {
    let mut ptr = char_ptr;
    loop {
        if ptr == char_end_ptr {
            break;
        }
        let chr = deref(ptr);
        if chr == Letters::NewLine || chr == Letters::CarriageReturn {
            break;
        }
        ptr = unsafe { ptr.add(1) };
    }
    ptr
}

// LF and CRLF. A lone CR is deliberately NOT a line break: it is a stray byte,
// and failing on it beats silently splitting a line somewhere unexpected.
fn strip_line_break(char_ptr: *const u8, char_end_ptr: *const u8) -> (bool, *const u8) {
    let (is_crlf, after) = strip_prefix_on_match(char_ptr, char_end_ptr, "\r\n");
    if is_crlf {
        return (true, after);
    }
    let (is_lf, after) = strip_prefix_on_match(char_ptr, char_end_ptr, "\n");
    if is_lf {
        return (true, after);
    }
    (false, char_ptr)
}

// Horizontal trivia: spaces, plus a `--` comment running to end of line.
//
// Comments are handled here rather than in skip_trivia so that a comment
// trailing actual code is eaten by the ordinary inter-token whitespace skip,
// which every parser already calls. The returned count is the number of
// SPACES before the first significant byte -- a comment contributes nothing to
// it, so a comment-only line is transparent to the indentation logic.
#[inline(never)]
fn skip_whitespaces(char_ptr: *const u8, char_end_ptr: *const u8) -> (u32, *const u8) {
    let mut depth = 0u32;
    let mut local_char_ptr = char_ptr;
    loop {
        if char_end_ptr == local_char_ptr {
            break;
        }
        let chr = deref(local_char_ptr);
        if chr == Letters::Whitespace {
            depth += 1;
            local_char_ptr = unsafe { local_char_ptr.add(1) };
            continue;
        }
        let (is_comment, after) = strip_prefix_on_match(local_char_ptr, char_end_ptr, "--");
        if is_comment {
            local_char_ptr = skip_to_line_end(after, char_end_ptr);
            continue;
        }
        break;
    }
    (depth, local_char_ptr)
}

pub fn skip_trivia(char_ptr: *const u8, char_end_ptr: *const u8) -> (u32, *const u8) {
    let mut depth = 0;
    let mut ptr = char_ptr;
    loop {
        if ptr == char_end_ptr {
            break;
        }
        let (depth_, next_ptr) = skip_whitespaces(ptr, char_end_ptr);
        ptr = next_ptr;
        depth = depth_;
        // The original dereferenced here without re-testing for the end, so a
        // source ending in spaces read past the buffer.
        if ptr == char_end_ptr {
            break;
        }
        let (is_break, after) = strip_line_break(ptr, char_end_ptr);
        if is_break {
            ptr = after;
            continue;
        } else {
            break;
        }
    }
    (depth, ptr)
}

// Scans for a tab anywhere in the source. DDL is indentation-sensitive and
// only counts spaces, so a tab would silently change block membership rather
// than being an error. The driver rejects the file instead.
pub fn find_tab(char_ptr: *const u8, char_end_ptr: *const u8) -> Option<*const u8> {
    let mut ptr = char_ptr;
    loop {
        if ptr == char_end_ptr {
            return None;
        }
        if deref(ptr) == Letters::Tab {
            return Some(ptr);
        }
        ptr = unsafe { ptr.add(1) };
    }
}

// demand delimiter presense, or look silly... (purely cosmetic)
fn any_delimiter_present(char_ptr: *const u8, char_end_ptr: *const u8) -> bool {
    if char_ptr == char_end_ptr {
        return true;
    }
    let char = deref(char_ptr);
    match char {
        Letters::NewLine | Letters::Whitespace | Letters::CarriageReturn | Letters::Tab => true,
        // A trailing comment ends the keyword just as well as a space does.
        Letters::Dash => {
            strip_prefix_on_match(char_ptr, char_end_ptr, "--").0
        }
        _ => false,
    }
}

fn try_parse_alphanum(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(AlphanumSpan, *const u8), ()> {
    let mut len = 0;
    let mut ptr = char_ptr;
    loop {
        if ptr == char_end_ptr {
            break;
        }
        let ch = deref(ptr);
        let is_lower_case_ascii = (Letters::a..=Letters::z).contains(&ch);
        let is_upper_case_ascii = (Letters::A..=Letters::Z).contains(&ch);
        let is_under_score = ch == Letters::Underscore;
        let is_number = (Letters::ZERO..=Letters::NINE).contains(&ch);
        let is_at_sign = ch == Letters::AT_SIGN;
        let valid_char =
            is_lower_case_ascii || is_upper_case_ascii || is_under_score || is_number || is_at_sign;
        if !valid_char {
            break;
        }
        len += 1;
        ptr = unsafe { ptr.add(1) };
    }
    if len == 0 {
        return Err(());
    }
    let ns = AlphanumSpan {
        byte_ptr: char_ptr,
        len,
    };
    Ok((ns, ptr))
}

fn digit_value(ch: u8) -> Option<u32> {
    match ch {
        b'0'..=b'9' => Some((ch - b'0') as u32),
        b'a'..=b'f' => Some((ch - b'a') as u32 + 10),
        b'A'..=b'F' => Some((ch - b'A') as u32 + 10),
        _ => None,
    }
}

/// Reads digits in `radix`, allowing `_` as a separator between them.
/// Returns the accumulated value, the digit count, and the new cursor.
/// A leading `_` is not a digit, so it will not start a run.
fn scan_digits(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    radix: u32,
) -> (u128, u32, *const u8) {
    let mut value: u128 = 0;
    let mut count = 0u32;
    let mut ptr = char_ptr;
    loop {
        if ptr == char_end_ptr {
            break;
        }
        let ch = deref(ptr);
        if ch == Letters::Underscore {
            // Only meaningful between digits; a trailing `_` is harmless.
            if count == 0 {
                break;
            }
            ptr = unsafe { ptr.add(1) };
            continue;
        }
        match digit_value(ch) {
            Some(d) if d < radix => {
                value = value.wrapping_mul(radix as u128).wrapping_add(d as u128);
                count += 1;
                ptr = unsafe { ptr.add(1) };
            }
            _ => break,
        }
    }
    (value, count, ptr)
}

fn radix_of(ch: u8) -> Option<u32> {
    match ch {
        b'h' | b'H' => Some(16),
        b'd' | b'D' => Some(10),
        b'o' | b'O' => Some(8),
        b'b' | b'B' => Some(2),
        _ => None,
    }
}

/// Numeric literals.
///
/// ```text
/// 8'hFF   4'b1010   16'd12   1'b0      -- sized, Verilog spelling
/// 0xFF    0b1010    0o17     123       -- unsized
/// 1_000   0xDEAD_BEEF                  -- `_` separates digits
/// 0.717                                -- float
/// ```
///
/// Before this existed there was no numeric lexer at all: `123` was an
/// identifier that a later pass noticed was all digits, and `0.717` parsed as
/// a member access on `0`. Neither could carry a width, which an HDL requires.
fn try_parse_number(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawNum, *const u8), ()> {
    if char_ptr == char_end_ptr {
        return Err(());
    }
    if digit_value(deref(char_ptr)).filter(|d| *d < 10).is_none() {
        return Err(());
    }

    // `0x` / `0b` / `0o` prefixes.
    let (is_zero, after_zero) = strip_prefix_on_match(char_ptr, char_end_ptr, "0");
    if is_zero && after_zero != char_end_ptr {
        let marker = deref(after_zero);
        let radix = match marker {
            b'x' | b'X' => Some(16),
            b'b' | b'B' => Some(2),
            b'o' | b'O' => Some(8),
            _ => None,
        };
        if let Some(radix) = radix {
            let digits_at = unsafe { after_zero.add(1) };
            let (value, count, tail) = scan_digits(digits_at, char_end_ptr, radix);
            if count == 0 {
                return Err(());
            }
            return Ok((
                RawNum::Int { span: span_between(char_ptr, tail), width: None, value },
                tail,
            ));
        }
    }

    // A plain decimal run. It is either the whole literal, the width of a
    // sized literal, or the whole part of a float.
    let (lead, lead_count, after_lead) = scan_digits(char_ptr, char_end_ptr, 10);
    if lead_count == 0 {
        return Err(());
    }

    // Sized: `<width>'<radix><digits>`.
    let (is_tick, after_tick) = strip_prefix_on_match(after_lead, char_end_ptr, "'");
    if is_tick && after_tick != char_end_ptr {
        let radix = match radix_of(deref(after_tick)) {
            Some(r) => r,
            None => return Err(()),
        };
        let digits_at = unsafe { after_tick.add(1) };
        let (value, count, tail) = scan_digits(digits_at, char_end_ptr, radix);
        if count == 0 {
            return Err(());
        }
        if lead > u32::MAX as u128 {
            return Err(());
        }
        return Ok((
            RawNum::Int {
                span: span_between(char_ptr, tail),
                width: Some(lead as u32),
                value,
            },
            tail,
        ));
    }

    // Float: `<whole>.<frac>`, and only when a digit actually follows the dot,
    // so that tuple access like `x.0` and ranges like `1..8` are unaffected.
    let (is_dot, after_dot) = strip_prefix_on_match(after_lead, char_end_ptr, ".");
    if is_dot && after_dot != char_end_ptr
        && digit_value(deref(after_dot)).filter(|d| *d < 10).is_some() {
            let (frac, _, tail) = scan_digits(after_dot, char_end_ptr, 10);
            return Ok((
                RawNum::Float {
                    span: span_between(char_ptr, tail),
                    whole: lead as u64,
                    frac: frac as u64,
                },
                tail,
            ));
        }

    Ok((
        RawNum::Int { span: span_between(char_ptr, after_lead), width: None, value: lead },
        after_lead,
    ))
}

fn span_between(start: *const u8, end: *const u8) -> AlphanumSpan {
    let len = (end as usize) - (start as usize);
    AlphanumSpan { byte_ptr: start, len: len as u32 }
}

fn try_parse_arg_type_qualifier(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(ArgTypeQualifier, *const u8), ()> {
    // A labelled block rather than a `loop` that always breaks. The two
    // compile the same and only one says what it is; the `loop` spelling
    // predates labelled blocks being stable.
    let qualifier = 'qualifier: {
        let (is_out, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "inout ");
        if is_out {
            char_ptr = new_ptr;
            break 'qualifier ArgTypeQualifier::Inout
        }
        let (is_in, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "out ");
        if is_in {
            char_ptr = new_ptr;
            break 'qualifier ArgTypeQualifier::Out
        }
        let (is_stream, new_ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, "stream ");
        if is_stream {
            let (is_in, ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, "in ");
            if is_in {
                char_ptr = ptr;
                break 'qualifier ArgTypeQualifier::StreamIn
            }
            let (is_out, ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, "out ");
            if is_out {
                char_ptr = ptr;
                break 'qualifier ArgTypeQualifier::StreamOut
            }
            return Err(());
        }
        let (is_buffer, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "buffer ");
        if is_buffer {
            let (is_in, ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, "in ");
            if is_in {
                char_ptr = ptr;
                break 'qualifier ArgTypeQualifier::BufferIn
            }
            let (is_out, ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, "out ");
            if is_out {
                char_ptr = ptr;
                break 'qualifier ArgTypeQualifier::BufferOut
            }
            return Err(());
        }
        char_ptr = new_ptr;
        break 'qualifier ArgTypeQualifier::In;
    };
    Ok((qualifier, char_ptr))
}

unsafe fn try_parse_type_expr(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawTypeExpr, *const u8), ()> {
    // `#[impl(lutram)] [T; n]`. The annotation binds to the array that follows,
    // so it is parsed here rather than at the declaration: it is part of what
    // the type IS, not a property of the name it is given.
    let (is_annotated, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "#[");
    if is_annotated {
        char_ptr = new_ptr;
        let (word, new_ptr) = try_parse_alphanum(char_ptr, char_end_ptr)?;
        char_ptr = new_ptr;
        // `impl` is the only annotation there is; anything else is a typo, and
        // silently ignoring it would silently ignore a memory shape request.
        let word_text = core::str::from_raw_parts(word.byte_ptr, word.len as usize);
        if word_text != "impl" {
            return Err(());
        }
        let (open, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
        if !open {
            return Err(());
        }
        char_ptr = new_ptr;
        let (kind, new_ptr) = try_parse_alphanum(char_ptr, char_end_ptr)?;
        char_ptr = new_ptr;
        let (close, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, ")]");
        if !close {
            return Err(());
        }
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (inner, new_ptr) = try_parse_type_expr(char_ptr, char_end_ptr)?;
        let (elem, len) = match inner {
            RawTypeExpr::Array(elem, len) => (elem, len),
            // `#[impl(bram)] i32` asks for a memory that holds one thing; the
            // annotation only means anything on an array.
            _ => return Err(()),
        };
        return Ok((RawTypeExpr::MemArray { elem, len, kind }, new_ptr));
    }

    let (is_arr_begin, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "[");
    if is_arr_begin {
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (ete, new_ptr) = try_parse_type_expr(char_ptr, char_end_ptr)?;
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (is_sep, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ";");
        if !is_sep {
            return Err(())
        }
        char_ptr = tail;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (expr, new_ptr) = try_parse_expr(char_ptr, char_end_ptr, 0)?;
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (is_sep, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "]");
        if !is_sep {
            return Err(())
        }
        char_ptr = new_ptr;
        let ty_expr = RawTypeExpr::Array(Box::new(ete), expr);
        return Ok((ty_expr, char_ptr))
    }
    let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = new_ptr;
    let (name_span, new_ptr) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = new_ptr;
    let ty_expr = RawTypeExpr::Ident(name_span);
    Ok((ty_expr, char_ptr))
}

unsafe fn parse_arg_tuple(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawArgDefTuple, *const u8), ()> {
    let (matched, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    if !matched {
        return Err(());
    }
    char_ptr = new_ptr;
    let mut entries = Vec::new();
    loop {
        let (_, new_ptr) = skip_trivia(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (arg_name, new_ptr) = try_parse_alphanum(char_ptr, char_end_ptr)?;
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (matched, new_ptr) = strip_prefix_on_match(char_ptr, char_end_ptr, ":");
        if !matched {
            return Err(());
        }
        char_ptr = new_ptr;
        let (_, new_ptr) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = new_ptr;
        let (argq, new_ptr) = try_parse_arg_type_qualifier(char_ptr, char_end_ptr)?;
        char_ptr = new_ptr;
        let outcome = try_parse_type_expr(char_ptr, char_end_ptr);
        let (ty_expr, new_ptr) = match outcome {
            Ok(val) => val,
            Err(_) => return Err(()),
        };
        // `= <expr>`: the value of a constant parameter.
        let (_, after_ty) = skip_whitespaces(new_ptr, char_end_ptr);
        let (has_default, after_eq) = strip_prefix_on_match(after_ty, char_end_ptr, "=");
        let mut new_ptr = new_ptr;
        let mut default = None;
        if has_default {
            let (_, tail) = skip_whitespaces(after_eq, char_end_ptr);
            let (expr, tail) = try_parse_expr(tail, char_end_ptr, 0)?;
            default = Some(expr);
            new_ptr = tail;
        }
        // we have everything we need to build arg entry
        let arg_entry = RawArgTupleEntry {
            arg_name,
            qualifier: argq,
            type_expr: ty_expr,
            default,
        };
        entries.push(arg_entry);
        let (_, new_ptr) = skip_whitespaces(new_ptr, char_end_ptr);
        let (is_comma, new_ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, ",");
        if is_comma {
            char_ptr = new_ptr;
            continue;
        }
        let (is_rparen, new_ptr) = strip_prefix_on_match(new_ptr, char_end_ptr, ")");
        if is_rparen {
            char_ptr = new_ptr;
            break;
        }
        return Err(());
    }
    let args = RawArgDefTuple { entries };
    Ok((args, char_ptr))
}

unsafe fn try_parse_inner_stmt(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    scope_depth: u32,
) -> Result<(InnerStmt, *const u8), ()> {
    // MIND THE ORDER!

    let maybe_var = try_parse_var_decl_stmt(char_ptr, char_end_ptr, scope_depth);
    match maybe_var {
        Ok((var_decl, tail_ptr)) => return Ok((InnerStmt::VarDecl(var_decl), tail_ptr)),
        Err(is_deep_match) => {
            if is_deep_match {
                return Err(());
            }
        }
    }
    match try_parse_match_stmt(char_ptr, char_end_ptr, scope_depth) {
        Ok((match_stmt, tail_ptr)) => return Ok((InnerStmt::MatchStmt(match_stmt), tail_ptr)),
        Err(is_deep_match) => {
            if is_deep_match {
                return Err(());
            }
        }
    };
    match try_parse_ite_stmt(char_ptr, char_end_ptr, scope_depth) {
        Ok((ite, tail)) => return Ok((InnerStmt::IfThenElse(ite), tail)),
        Err(is_deep_match) => {
            if is_deep_match {
                return Err(());
            }
        }
    }
    match try_parse_loop_stmt(char_ptr, char_end_ptr, scope_depth) {
        Ok((loop_stmt, tail)) => return Ok((InnerStmt::Loop(loop_stmt), tail)),
        Err(deep) => {
            if deep {
                return Err(());
            }
        }
    }
    if let Ok(tail) = try_parse_break_stmt(char_ptr, char_end_ptr) {
        return Ok((InnerStmt::Break, tail));
    }
    match try_parse_for_loop(char_ptr, char_end_ptr, scope_depth) {
        Ok((for_loop, tail)) => return Ok((InnerStmt::ForLoopStmt(for_loop), tail)),
        Err(deep) => {
            if deep {
                return Err(());
            }
        }
    }
    match try_parse_return_stmt(char_ptr, char_end_ptr, scope_depth) {
        Ok((return_stmt, tail)) => return Ok((InnerStmt::ReturnStmt(return_stmt), tail)),
        Err(deep) => {
            if deep {
                return Err(());
            }
        }
    }

    // this should be last always
    if let Ok((expr, tail)) = try_parse_expr(char_ptr, char_end_ptr, scope_depth) {
        return Ok((InnerStmt::ExprStmt(expr), tail));
    }

    Err(())
}

unsafe fn try_parse_return_stmt(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    scope_depth: u32,
) -> Result<(Option<RawExpr>, *const u8), bool> {
    let (is_return, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "return");
    if !is_return {
        return Err(false);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    match try_parse_expr(char_ptr, char_end_ptr, scope_depth) {
        Ok((ret_expr, tail)) => Ok((Some(ret_expr), tail)),
        Err(_) => Ok((None, char_ptr)),
    }
}

unsafe fn try_parse_for_loop(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    depth: u32,
) -> Result<(ForLoopStmt, *const u8), bool> {
    let (is_for, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "for");
    if !is_for {
        return Err(false);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (binding, tail) = match try_parse_alphanum(char_ptr, char_end_ptr) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (is_in, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "in");
    if !is_in {
        return Err(true);
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (target, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (body, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;
    Ok((
        ForLoopStmt {
            binding,
            target,
            body,
        },
        char_ptr,
    ))
}

fn try_parse_break_stmt(char_ptr: *const u8, char_end_ptr: *const u8) -> Result<*const u8, ()> {
    let (is_break, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "break");
    // Every other keyword statement demands a delimiter; without it here,
    // `breakfast` lexed as `break` followed by the expression `fast`.
    if is_break && any_delimiter_present(tail, char_end_ptr) {
        return Ok(tail);
    }
    Err(())
}

unsafe fn try_parse_loop_stmt(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    depth: u32,
) -> Result<(LoopStmt, *const u8), bool> {
    let (is_loop, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "loop");
    if !is_loop {
        return Err(false);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (body, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;
    let rs = LoopStmt { repeat_expr: body };
    Ok((rs, char_ptr))
}

unsafe fn try_parse_ite_stmt(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    depth: u32,
) -> Result<(ITEStmt, *const u8), bool> {
    let (is_ite, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "if");
    if !is_ite {
        return Err(false);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (cond, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    // now we need to check 'then' part
    let (then_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
    let is_linebreak = then_depth != 0;
    if is_linebreak {
        // on line break we must enforce that 'then' is at the same depth as 'if'
        let at_same_depth = depth == then_depth;
        if !at_same_depth {
            return Err(true);
        }
        char_ptr = tail;
    } else {
        // this 'then' expr is on the same line as if
    }
    let (is_then, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "then");
    if !is_then {
        return Err(true);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (true_case, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
        Ok(val) => val,
        Err(_) => return Err(true),
    };
    char_ptr = tail;

    // else part
    let mut false_case = None;
    let (else_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
    let is_linebreak = else_depth != 0;

    // A token at a different indentation belongs to an enclosing block, so
    // this `if` simply has no `else`. This used to `return Err(true)` -- a
    // committed failure -- which made dedenting two levels at once after a
    // nested bare `if` a hard parse error for the whole declaration, with the
    // blame landing on the `fun` or `process` keyword.
    let could_be_else = !is_linebreak || else_depth == depth;
    if !could_be_else {
        let rs = ITEStmt {
            condition: cond,
            then_case: true_case,
            else_case: None,
        };
        return Ok((rs, char_ptr));
    }

    let (is_else, tail) = strip_prefix_on_match(tail, char_end_ptr, "else");
    if is_else {
        char_ptr = tail;
        if !any_delimiter_present(char_ptr, char_end_ptr) {
            return Err(true)
        };
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (on_false_expr, tail) = match try_parse_expr(char_ptr, char_end_ptr, depth) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        char_ptr = tail;
        false_case = Some(on_false_expr);
    }

    let rs = ITEStmt {
        condition: cond,
        then_case: true_case,
        else_case: false_case,
    };
    Ok((rs, char_ptr))
}

unsafe fn parse_proc_inner_stmt(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    depth: u32,
) -> Result<(InnerStmt, *const u8), ()> {
    try_parse_inner_stmt(char_ptr, char_end_ptr, depth)
}

unsafe fn try_parse_match_stmt(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    match_depth: u32,
) -> Result<(MatchStmt, *const u8), bool> {
    let (is_match, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "match");
    if !is_match {
        return Err(false);
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(true)
    };
    let mut scruts = Vec::new();
    loop {
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (scrut, tail) = match try_parse_expr(char_ptr, char_end_ptr, match_depth) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        char_ptr = tail;
        scruts.push(scrut);
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (another_scrut, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ",");
        if another_scrut {
            char_ptr = tail;
            continue;
        }
        break;
    }

    // probe depth
    let (anchore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = match_depth < anchore_depth;
    if !inbound {
        return Err(true);
    } // no arm match

    let mut arms = Vec::new();
    loop {
        let (arm_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = arm_depth == anchore_depth;
        if !same_depth {
            break;
        }
        char_ptr = tail;
        let (arm, tail) = match try_parse_match_arm(char_ptr, char_end_ptr, arm_depth) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        char_ptr = tail;
        arms.push(arm)
    }
    let rs = MatchStmt {
        scrutinees: scruts,
        cases: arms,
    };
    Ok((rs, char_ptr))
}

unsafe fn try_parse_match_arm(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    arm_depth: u32,
) -> Result<(MatchArm, *const u8), ()> {
    let (bindings, tail) = try_parse_match_arm_lhs(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (is_lhr_rhs_delim, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "=>");
    if !is_lhr_rhs_delim {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (rhs, tail) = try_parse_expr(char_ptr, char_end_ptr, arm_depth)?;
    char_ptr = tail;

    let rs = MatchArm {
        binding_patterns: bindings,
        rhs,
    };
    Ok((rs, char_ptr))
}

fn try_parse_match_arm_lhs(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(Vec<BindingPattern>, *const u8), ()> {
    let mut bindings = Vec::new();
    loop {
        // One scrutinee position, which may offer several alternatives joined
        // by `|`. `k2g_decode.sv` groups opcodes this way eight times over, and
        // without it a 64-way decode has to choose between duplicated bodies
        // and a `_` that switches exhaustiveness checking off.
        let (first, tail) = try_parse_case_pattern(char_ptr, char_end_ptr)?;
        char_ptr = tail;
        let mut alternatives = vec![first];
        loop {
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            // `||` is the logical operator, never a pattern separator.
            let (is_log_or, _) = strip_prefix_on_match(tail, char_end_ptr, "||");
            if is_log_or {
                break;
            }
            let (is_alt, tail) = strip_prefix_on_match(tail, char_end_ptr, "|");
            if !is_alt {
                break;
            }
            char_ptr = tail;
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            let (next, tail) = try_parse_case_pattern(char_ptr, char_end_ptr)?;
            char_ptr = tail;
            alternatives.push(next);
        }
        let has_alternatives = alternatives.len() > 1;
        if has_alternatives {
            bindings.push(BindingPattern::AnyOf(alternatives));
        } else {
            bindings.push(alternatives.pop().expect("at least one pattern"));
        }

        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (more, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ",");
        if more {
            char_ptr = tail;
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            continue;
        }
        break;
    }
    Ok((bindings, char_ptr))
}

fn try_parse_case_pattern(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(BindingPattern, *const u8), ()> {
    // is enum case ref?
    let (is_leading_dot, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ".");
    if is_leading_dot {
        char_ptr = tail;
        let (case_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        // has subbinding?
        if let Ok((bind_name, tail)) = try_parse_alphanum(char_ptr, char_end_ptr) {
            char_ptr = tail;
            return Ok((
                BindingPattern::EnumCase {
                    base: case_name,
                    subbinding: Some(bind_name),
                },
                char_ptr,
            ));
        }
        return Ok((
            BindingPattern::EnumCase {
                base: case_name,
                subbinding: None,
            },
            char_ptr,
        ));
    }

    // gotta be alpha num then
    let (case_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    Ok((BindingPattern::Alphanum(case_name), tail))
}

unsafe fn try_parse_var_decl_stmt(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
) -> Result<(VarDeclStmt, *const u8), bool> {
    let is_mutable;
    let (is_imut, tail1) = strip_prefix_on_match(char_ptr, char_end_ptr, "let ");
    let (is_mut, tail2) = strip_prefix_on_match(char_ptr, char_end_ptr, "var ");
    if is_imut {
        char_ptr = tail1;
        is_mutable = false;
    } else if is_mut {
        char_ptr = tail2;
        is_mutable = true;
    } else {
        return Err(false);
    }
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    // `let (val, ok) = @try_rcv(p)`. A non-blocking receive answers with the
    // item and whether there was one, and there is no way to use it without
    // taking both (desc.md:148).
    let mut rest: Vec<AlphanumSpan> = Vec::new();
    let (is_tuple, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    let var_name;
    if is_tuple {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (first, tail) = match try_parse_alphanum(char_ptr, char_end_ptr) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        var_name = first;
        char_ptr = tail;
        loop {
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            let (more, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ",");
            if !more {
                break;
            }
            char_ptr = tail;
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            let (next, tail) = match try_parse_alphanum(char_ptr, char_end_ptr) {
                Ok(val) => val,
                Err(_) => return Err(true),
            };
            rest.push(next);
            char_ptr = tail;
        }
        let (closed, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ")");
        if !closed {
            return Err(true);
        }
        char_ptr = tail;
    } else {
        let (name, tail) = match try_parse_alphanum(char_ptr, char_end_ptr) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        var_name = name;
        char_ptr = tail;
    }
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (is_ty_ascr, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ":");
    let ty_expr: Option<RawTypeExpr>;
    if is_ty_ascr {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (raw_ty_expr, tail) = match try_parse_type_expr(char_ptr, char_end_ptr) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        char_ptr = tail;
        // collect type
        ty_expr = Some(raw_ty_expr);
        //
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
    } else {
        // type is not given, gotta derive it!
        ty_expr = None;
    }
    let (is_assigned, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "=");
    let assign_val;
    if is_assigned {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (assign_val_, tail) = match try_parse_expr(char_ptr, char_end_ptr, anchor_depth) {
            Ok(val) => val,
            Err(_) => return Err(true),
        };
        char_ptr = tail;
        assign_val = Some(assign_val_);
    } else {
        assign_val = None;
    }
    let result = VarDeclStmt {
        is_mutable,
        name: var_name,
        rest,
        ty_expr,
        assign_val,
    };
    Ok((result, char_ptr))
}

unsafe fn try_parse_invocation_tuple(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(Vec<RawExpr>, *const u8), ()> {
    let (is_paren_begin, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    if !is_paren_begin {
        return Err(());
    }
    char_ptr = tail;
    let mut args = Vec::new();
    loop {
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        // An argument list may continue on the next line, whether the break
        // comes after the open paren or after a comma. A call with eight
        // operands is unreadable on one line, and every port list in the
        // SystemVerilog this replaces is wrapped.
        //
        // The continuation must be indented PAST the statement that opened the
        // call. That is what keeps it distinguishable from the other thing a
        // line break can start here -- an indented block -- and it is the same
        // test the closing paren below makes, one level in the other
        // direction.
        let (depth, tail_) = skip_trivia(char_ptr, char_end_ptr);
        let continues_on_next_line = depth > parent_depth;
        if continues_on_next_line {
            char_ptr = tail_;
        }
        let (is_paren_end, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ")");
        if is_paren_end {
            char_ptr = tail;
            break;
        }
        let (arg, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
        char_ptr = tail;
        args.push(arg);
        let (is_comma, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ",");
        if is_comma {
            char_ptr = tail;
        }
        {
            // special case to make it valid for closing paren to appear on newline e.g. call(...\n)
            let (rparen_depth, tail_) = skip_trivia(char_ptr, char_end_ptr);
            let (is_rparen, tail_) = strip_prefix_on_match(tail_, char_end_ptr, ")");
            let is_linebreak = rparen_depth != 0;
            let special_case = is_linebreak && is_rparen;
            if special_case {
                let is_correct_indent = rparen_depth == parent_depth;
                if !is_correct_indent {
                    return Err(());
                }
                char_ptr = tail_;
                break;
            }
        }
    }
    Ok((args, char_ptr))
}

unsafe fn parse_indent_guided_block(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(StmtBlock, *const u8), ()> {
    let _nesting = match enter_nesting() {
        Some(g) => g,
        None => return Err(()),
    };
    // probe depth
    let (anchore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = parent_depth < anchore_depth;
    if !inbound {
        return Err(());
    } // the block must be deeper than parent
    let mut stmts = Vec::new();
    loop {
        let (depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = depth == anchore_depth;
        if !same_depth {
            break;
        }
        char_ptr = tail;
        let (stmt, tail) = try_parse_inner_stmt(char_ptr, char_end_ptr, anchore_depth)?;
        char_ptr = tail;
        stmts.push(stmt);
    }
    let rs = StmtBlock { components: stmts };
    Ok((rs, char_ptr))
}

unsafe fn try_parse_multiline_string(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(StrLiteral, *const u8), bool> {
    let (ancore_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
    if ancore_depth == 0 {
        return Err(false);
    }
    let indented = ancore_depth > parent_depth;
    if !indented {
        return Err(false);
    }
    char_ptr = tail;
    let mut pieces = Vec::new();
    let (str, tail) = try_parse_line_string(char_ptr, char_end_ptr)?;
    pieces.push(str);
    char_ptr = tail;
    loop {
        let (depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        if depth != ancore_depth {
            break;
        }
        char_ptr = tail;
        if let Ok((str, tail)) = try_parse_line_string(char_ptr, char_end_ptr) {
            pieces.push(str);
            char_ptr = tail;
        } else {
            break;
        }
    }
    if pieces.is_empty() {
        return Err(false);
    }
    Ok((StrLiteral { pieces }, char_ptr))
}
unsafe fn try_parse_line_string(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(StrSpan, *const u8), bool> {
    let (is_str, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "\"");
    if !is_str {
        return Err(false);
    }
    let bytes_start_addr = tail;
    let mut ptr = tail;
    let out;
    loop {
        let text_ended = ptr == char_end_ptr;
        if text_ended {
            return Err(true);
        }
        let str_ended = deref(ptr) == b'\"';
        let len = ptr as usize - bytes_start_addr as usize;
        ptr = ptr.add(1);
        if str_ended {
            out = StrSpan {
                start_ptr: bytes_start_addr,
                len,
            };
            break;
        }
    }
    Ok((out, ptr))
}

#[derive(Debug, Clone)]
pub struct StrLiteral {
    pub pieces: Vec<StrSpan>,
}
#[derive(Clone, Copy)]
pub struct StrSpan {
    pub start_ptr: *const u8,
    pub len: usize,
}

impl StrSpan {
    /// The text between the quotes, borrowed from the source.
    pub fn as_str<'a>(&self) -> &'a str {
        unsafe { core::str::from_raw_parts(self.start_ptr, self.len) }
    }
}


/// `if <cond> then <expr> else <expr>`, all on one logical line.
///
/// Deliberately separate from `try_parse_ite_stmt`: that form takes indented
/// statement blocks and allows the `else` to be absent, neither of which is
/// meaningful for something that has to produce a value.
unsafe fn try_parse_ternary(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(RawExpr, *const u8), ()> {
    let (is_if, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "if");
    if !is_if {
        return Err(());
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(());
    }
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    let (cond, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
    char_ptr = tail;

    // A line break before `then` is allowed so a long chain can be wrapped.
    let (_, tail) = skip_trivia(char_ptr, char_end_ptr);
    let (is_then, tail) = strip_prefix_on_match(tail, char_end_ptr, "then");
    if !is_then {
        return Err(());
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(());
    }
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    let (then_e, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
    char_ptr = tail;

    let (_, tail) = skip_trivia(char_ptr, char_end_ptr);
    let (is_else, tail) = strip_prefix_on_match(tail, char_end_ptr, "else");
    if !is_else {
        return Err(());
    }
    char_ptr = tail;
    if !any_delimiter_present(char_ptr, char_end_ptr) {
        return Err(());
    }
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    let (else_e, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
    char_ptr = tail;

    let rs = RawExpr::Ternary {
        cond: Box::new(cond),
        then_e: Box::new(then_e),
        else_e: Box::new(else_e),
    };
    Ok((rs, char_ptr))
}


/// Skips spaces, and a line break too when what follows is indented deeper
/// than the statement being parsed.
///
/// That is what lets a long expression wrap. The rule is unambiguous because
/// a new statement always begins at the statement's own depth, never deeper --
/// so anything deeper can only be a continuation. Used after an infix
/// operator, where an operand must follow and a bare newline would otherwise
/// end the expression mid-way and leave the rest to be misparsed.
fn skip_expr_continuation(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> *const u8 {
    let (_, after_spaces) = skip_whitespaces(char_ptr, char_end_ptr);
    let (is_break, _) = strip_line_break(after_spaces, char_end_ptr);
    if !is_break {
        return after_spaces;
    }
    let (depth, after_break) = skip_trivia(after_spaces, char_end_ptr);
    let is_continuation = depth > parent_depth;
    if is_continuation { after_break } else { after_spaces }
}

unsafe fn try_parse_expr_1(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(RawExpr, *const u8), ()> {
    // `if c then a else b` as a value.
    //
    // This is also what makes `else if` work: the else branch is an
    // expression, so it can be another `if`. The decoder is full of the
    // SystemVerilog form this replaces -- `(lb == LB_PUC8) ? RDT_U8 : ...`.
    if let Ok((expr, tail)) = try_parse_ternary(char_ptr, char_end_ptr, parent_depth) {
        return Ok((expr, tail));
    }

    // parened expr?
    let (is_paren_begin, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    if is_paren_begin {
        char_ptr = tail;
        let (expr, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
        char_ptr = tail;
        let (is_paren_end, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ")");
        if !is_paren_end {
            return Err(());
        }
        char_ptr = tail;
        return Ok((expr, char_ptr));
    }

    // unary prefix operator?
    //
    // `-` here is unambiguous: a `--` would already have been eaten as a
    // comment by the whitespace skip that precedes every atom.
    macro prefix_op_rule($sym:expr, $op:expr) {
        let (is_match, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, $sym);
        if is_match {
            char_ptr = tail;
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            let (operand, tail) = try_parse_expr_2(char_ptr, char_end_ptr, parent_depth)?;
            return Ok((
                RawExpr::Unary { op: $op, operand: Box::new(operand) },
                tail,
            ));
        }
    }
    // `!=` is an infix operator, so it must not be read as `!` applied to `=`.
    let (is_ne, _) = strip_prefix_on_match(char_ptr, char_end_ptr, "!=");
    if !is_ne {
        prefix_op_rule!("!", UnaryOp::LogNot);
    }
    prefix_op_rule!("~", UnaryOp::BitNot);
    prefix_op_rule!("-", UnaryOp::Neg);

    // line string literal?
    match try_parse_line_string(char_ptr, char_end_ptr) {
        Ok((span, tail)) => {
            char_ptr = tail;
            return Ok((
                RawExpr::StrLiteral(StrLiteral { pieces: vec![span] }),
                char_ptr,
            ));
        }
        Err(deep) => {
            if deep {
                return Err(());
            }
        }
    }

    // numeric literal? Probed before identifiers because try_parse_alphanum
    // accepts leading digits, which is how int literals used to be recognised.
    if let Ok((num, tail)) = try_parse_number(char_ptr, char_end_ptr) {
        return Ok((RawExpr::NumLiteral(num), tail));
    }

    // lets try to parse just letters first, most common case
    let (ident, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;

    let result = RawExpr::AnumSpan(ident);
    Ok((result, char_ptr))
}

unsafe fn try_parse_expr_2(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(RawExpr, *const u8), ()> {
    let mut expr;
    let (head, tail) = try_parse_expr_1(char_ptr, char_end_ptr, parent_depth)?;
    char_ptr = tail;
    expr = head;

    'inner: loop {
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;

        // function call?
        let (is_arg_tuple_begin, _) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
        if is_arg_tuple_begin {
            let (args, tail) = try_parse_invocation_tuple(char_ptr, char_end_ptr, parent_depth)?;
            expr = RawExpr::Call {
                base: Box::new(expr),
                args,
            };
            char_ptr = tail;
            continue 'inner;
        }

        let (is_dot, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ".");
        let (is_second_dot, _) = strip_prefix_on_match(tail, char_end_ptr, ".");
        if is_dot && !is_second_dot {
            char_ptr = tail;
            // postfix operator?
            let (is_postfix_bitinverse, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "~");
            if is_postfix_bitinverse {
                char_ptr = tail;
                expr = RawExpr::Postfix {
                    base: Box::new(expr),
                    op: PostfixOp::Tilda,
                };
                continue 'inner;
            }

            // member access
            let (ident, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
            char_ptr = tail;
            expr = RawExpr::MemberAccess {
                base: Box::new(expr),
                field_name: ident,
            };
            continue 'inner;
        }

        // subscript access?
        let (is_subscript_begin, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "[");
        if is_subscript_begin {
            char_ptr = tail;
            let (index, tail) = try_parse_expr(char_ptr, char_end_ptr, parent_depth)?;
            char_ptr = tail;
            let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
            char_ptr = tail;
            let (is_subscript_end, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "]");
            if !is_subscript_end {
                // unexpected parse
                return Err(());
            }
            char_ptr = tail;
            expr = RawExpr::SubscriptAccess(Box::new(SubscriptAccess {
                base: expr,
                index,
            }));
            continue 'inner;
        }

        break 'inner;
    }

    Ok((expr, char_ptr))
}

unsafe fn try_parse_expr(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    parent_depth: u32,
) -> Result<(RawExpr, *const u8), ()> {
    let _nesting = match enter_nesting() {
        Some(g) => g,
        None => return Err(()),
    };
    // An expression starting at a line break is a statement block.
    //
    // This probed for a bare LF, so on a CRLF file it saw the CR, decided the
    // expression was inline, and every construct whose body is an indented
    // block -- match arms above all -- failed to parse. Both paths below go
    // through skip_trivia, which handles either ending, so the probe just had
    // to agree with them.
    let (is_block_start, _) = strip_line_break(char_ptr, char_end_ptr);
    if is_block_start {
        // multiline string?
        match try_parse_multiline_string(char_ptr, char_end_ptr, parent_depth) {
            Ok((str, tail)) => return Ok((RawExpr::StrLiteral(str), tail)),
            Err(deep) => {
                if deep {
                    return Err(());
                }
            }
        }
        // stmt block
        let (block, tail) = parse_indent_guided_block(char_ptr, char_end_ptr, parent_depth)?;
        char_ptr = tail;
        return Ok((RawExpr::StmtBlock(block), char_ptr));
    }

    let mut pieces = Vec::new();
    let (head, tail) = try_parse_expr_2(char_ptr, char_end_ptr, parent_depth)?;
    char_ptr = tail;
    pieces.push(InfixExprComponent::Subexpr(head));

    loop {
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;

        // expr with infix ops?
        macro infix_op_match_rule($sym:expr, $se_kind:expr) {
            let (is_match, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, $sym);
            if is_match {
                char_ptr = tail;
                pieces.push(InfixExprComponent::Basic($se_kind));
                char_ptr = skip_expr_continuation(char_ptr, char_end_ptr, parent_depth);
                let (piece, tail) = try_parse_expr_2(char_ptr, char_end_ptr, parent_depth)?;
                char_ptr = tail;
                pieces.push(InfixExprComponent::Subexpr(piece));
                continue;
            }
        }
        // STRICTLY LONGEST FIRST. Every operator that is a prefix of another
        // must be probed after it, or the longer one can never match: `<`
        // before `<=` would make `a <= b` parse as `a < (= b)`.
        infix_op_match_rule!("<<=", BasicInfixOp::Assign(AssignStmtKind::ShlAssign));
        infix_op_match_rule!(">>=", BasicInfixOp::Assign(AssignStmtKind::ShrAssign));
        infix_op_match_rule!("+=", BasicInfixOp::Assign(AssignStmtKind::AddAssign));
        infix_op_match_rule!("-=", BasicInfixOp::Assign(AssignStmtKind::SubAssign));
        infix_op_match_rule!("*=", BasicInfixOp::Assign(AssignStmtKind::MulAssign));
        infix_op_match_rule!("/=", BasicInfixOp::Assign(AssignStmtKind::DivAssign));
        infix_op_match_rule!("%=", BasicInfixOp::Assign(AssignStmtKind::ModAssign));
        infix_op_match_rule!("&=", BasicInfixOp::Assign(AssignStmtKind::AndAssign));
        infix_op_match_rule!("|=", BasicInfixOp::Assign(AssignStmtKind::OrAssign));
        infix_op_match_rule!("^=", BasicInfixOp::Assign(AssignStmtKind::XorAssign));
        infix_op_match_rule!("~=", BasicInfixOp::Assign(AssignStmtKind::InvertAssign));
        infix_op_match_rule!("==", BasicInfixOp::EqEq);
        infix_op_match_rule!("!=", BasicInfixOp::NotEq);
        infix_op_match_rule!("<=", BasicInfixOp::LtEq);
        infix_op_match_rule!(">=", BasicInfixOp::GtEq);
        infix_op_match_rule!("&&", BasicInfixOp::AmpAmp);
        infix_op_match_rule!("||", BasicInfixOp::VBarVBar);
        infix_op_match_rule!("<<", BasicInfixOp::DoubleBraketLeft);
        infix_op_match_rule!(">>", BasicInfixOp::DoubleBraketRight);
        infix_op_match_rule!("**", BasicInfixOp::StarStar);
        infix_op_match_rule!("..", BasicInfixOp::DotDot);
        infix_op_match_rule!("+", BasicInfixOp::Plus);
        infix_op_match_rule!("-", BasicInfixOp::Minus);
        infix_op_match_rule!("*", BasicInfixOp::Star);
        infix_op_match_rule!("/", BasicInfixOp::Slash);
        infix_op_match_rule!("%", BasicInfixOp::Percent);
        infix_op_match_rule!("&", BasicInfixOp::Ampersand);
        infix_op_match_rule!("|", BasicInfixOp::VBar);
        infix_op_match_rule!("^", BasicInfixOp::Caret);
        infix_op_match_rule!("<", BasicInfixOp::Lt);
        infix_op_match_rule!(">", BasicInfixOp::Gt);
        infix_op_match_rule!("=", BasicInfixOp::Assign(AssignStmtKind::PlainAssign));

        // no match then
        break;
    }

    Ok((RawExpr::InfixExpr { pieces }, char_ptr))
}

unsafe fn try_parse_sequence_inner_stmt(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
) -> Result<(RawSeqInnerStmt, *const u8), ()> {
    let (separator, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "|||");
    if separator {
        Ok((RawSeqInnerStmt::SegmentSeparator, tail))
    } else {
        let (stmt, tail) = try_parse_inner_stmt(char_ptr, char_end_ptr, anchor_depth)?;
        Ok((RawSeqInnerStmt::Stmt(stmt), tail))
    }
}
/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_enum_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawEnumDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "enum");
    if !matched {
        return Err(())
    }
    char_ptr = tail;
    // todo: err if we got no spaces between kw and name
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (enum_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    // Optional explicit tag width: `enum arith_e: i2`. A newline stops the
    // whitespace skip, so a plain `enum Name` cannot match this by accident.
    let (has_tag_type, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ":");
    let tag_type = if has_tag_type {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (ty, tail) = try_parse_type_expr(char_ptr, char_end_ptr)?;
        char_ptr = tail;
        Some(ty)
    } else {
        None
    };

    let (body_ancore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = anchor_depth < body_ancore_depth;
    if !inbound {
        return Err(());
    } // an enum with no variants is invalid

    let mut fields = Vec::new();
    loop {
        let (field_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = field_depth == body_ancore_depth;
        if !same_depth {
            // body ended
            break;
        }
        char_ptr = tail;
        let (f, tail) = match try_parse_enum_field(char_ptr, char_end_ptr, body_ancore_depth) {
            Ok(v) => v,
            Err(()) => {
                note_body_fail(fail, char_ptr, "enum");
                return Err(());
            }
        };
        if !line_is_finished(tail, char_end_ptr) {
            note_body_fail(fail, tail, "enum");
            return Err(());
        }
        char_ptr = tail;
        fields.push(f);
    }
    let rs = RawEnumDecl {
        name: enum_name,
        tag_type,
        fields,
    };
    Ok((rs, char_ptr))
}
/// One enum variant. Three forms:
///
/// ```text
/// ARITH_ADD              -- discriminant follows the previous variant
/// LB_EP1 = 6'b111111     -- explicit, which the generated k2g_pkg needs
/// Some: i32              -- payload; parsed, rejected later
/// ```
unsafe fn try_parse_enum_field(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    depth: u32,
) -> Result<(RawEnumField, *const u8), ()>  {
    let (field_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    // `Read(addr_t)` -- parenthesised, because that is how the value is built
    // (`Read(a)`) and how the pattern reads (`Read(a) =>`). A colon here would
    // be a third spelling of one idea, and would look like the tag width in
    // the line above it.
    let (has_payload, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    let payload = if has_payload {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (ty, tail) = try_parse_type_expr(char_ptr, char_end_ptr)?;
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (closed, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ")");
        if !closed {
            return Err(());
        }
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        Some(ty)
    } else {
        None
    };

    // `==` is a comparison, not a discriminant, so it must not match here.
    let (is_eq_eq, _) = strip_prefix_on_match(char_ptr, char_end_ptr, "==");
    let (has_discriminant, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "=");
    let discriminant = if has_discriminant && !is_eq_eq {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (expr, tail) = try_parse_expr(char_ptr, char_end_ptr, depth)?;
        char_ptr = tail;
        Some(expr)
    } else {
        None
    };

    let f = RawEnumField { name: field_name, discriminant, payload };
    Ok((f, char_ptr))
}
/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_struct_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawStructDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "struct ");
    if !matched {
        return Err(())
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (struct_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    let (body_ancore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = anchor_depth < body_ancore_depth;
    if !inbound {
        return Err(());
    } // no field struct is invalid

    let mut fields = Vec::new();
    loop {
        let (field_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = field_depth == body_ancore_depth;
        if !same_depth {
            // body ended
            break;
        }
        char_ptr = tail;
        let (f, tail) = match try_parse_struct_field(char_ptr, char_end_ptr) {
            Ok(v) => v,
            Err(()) => {
                note_body_fail(fail, char_ptr, "struct");
                return Err(());
            }
        };
        if !line_is_finished(tail, char_end_ptr) {
            note_body_fail(fail, tail, "struct");
            return Err(());
        }
        char_ptr = tail;
        fields.push(f);
    }
    let rs = RawStructDecl {
        name: struct_name,
        fields,
    };
    Ok((rs, char_ptr))
}
unsafe fn try_parse_struct_field(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawStructField, *const u8), ()>  {
    let (field_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (is_colon, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ":");
    if !is_colon {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (ty, tail) = try_parse_type_expr(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let f = RawStructField { name: field_name, field_type: ty };
    Ok((f, char_ptr))
}

/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_sequence_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawSequenceDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "sequence ");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (proc_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let outcome = parse_arg_tuple(char_ptr, char_end_ptr);
    let (args, tail) = match outcome {
        Ok(val) => val,
        Err(_) => return Err(()),
    };
    char_ptr = tail;
    let (body_ancore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = anchor_depth < body_ancore_depth;
    if !inbound {
        return Err(());
    } // no stmt process is invalid

    let mut stmts = Vec::new();
    loop {
        let (stmt_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = stmt_depth == body_ancore_depth;
        if !same_depth {
            // body ended
            break;
        }
        char_ptr = tail;

        let (inner_stmt, tail) =
            match try_parse_sequence_inner_stmt(char_ptr, char_end_ptr, body_ancore_depth) {
                Ok(v) => v,
                Err(()) => {
                    note_body_fail(fail, char_ptr, "sequence");
                    return Err(());
                }
            };
        stmts.push(inner_stmt);
        char_ptr = tail;
    }
    let res = RawSequenceDecl {
        name: proc_name,
        args,
        body: stmts,
    };
    Ok((res, char_ptr))
}

/// `graph Name (ports)` and its body.
///
/// Shaped like `try_parse_sequence_decl`: header, then statements indented
/// past the header's own column.
/// Whether the rest of this line is blank or a comment.
///
/// A body item that parses but does not reach the end of its line has left
/// something behind: `B ~~ C` in an enum parses `B`, stops, and the leftover
/// falls out of the declaration to be reported as top-level garbage with a
/// note about top-level declarations. Checking here keeps the complaint where
/// the reader is looking.
///
/// Only for line-oriented bodies -- enums, structs, graphs. A statement may
/// legitimately span lines, and this would call the second one leftovers.
unsafe fn line_is_finished(mut char_ptr: *const u8, char_end_ptr: *const u8) -> bool {
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    if char_ptr >= char_end_ptr {
        return true;
    }
    let (is_comment, _) = strip_prefix_on_match(char_ptr, char_end_ptr, "--");
    if is_comment {
        return true;
    }
    let here = deref(char_ptr);
    here == 10 || here == 13
}

// ---- recursion depth ------------------------------------------------------
//
// The parser is recursive descent, so nesting in the source is nesting on the
// stack: `((((a))))` recurses once per parenthesis and an indented block
// recurses once per level. At about 800 levels that overflows the stack, which
// is not a panic -- it is the process dying with no diagnostic, and
// `catch_unwind` cannot see it. The fuzzer found it in a minute; no valid
// program goes anywhere near it, which is why nothing had.
//
// A counter rather than a parameter because the recursion runs through
// twenty-one call sites in two separate cycles (expressions, and statement
// blocks), and threading a depth through all of them to be checked in two
// places is a lot of signature for one number.

use std::cell::Cell;

thread_local! {
    static NESTING: Cell<u32> = const { Cell::new(0) };
    /// Set when the limit is hit, so the failure reports as what it is rather
    /// than as whatever token happened to be next.
    static TOO_DEEP: Cell<bool> = const { Cell::new(false) };
}

/// How deep the source may nest.
///
/// Well under where the stack gives out, and far past anything a person
/// writes: the deepest expression in examples/ is six.
pub const NESTING_LIMIT: u32 = 96;

/// Decrements on the way out, which matters because the functions it guards
/// return early from dozens of places.
pub struct Nesting;

impl Drop for Nesting {
    fn drop(&mut self) {
        NESTING.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// `None` when the limit is reached; the caller fails the parse.
fn enter_nesting() -> Option<Nesting> {
    NESTING.with(|n| {
        let depth = n.get();
        if depth >= NESTING_LIMIT {
            TOO_DEEP.with(|f| f.set(true));
            return None;
        }
        n.set(depth + 1);
        Some(Nesting)
    })
}

fn reset_nesting() {
    NESTING.with(|n| n.set(0));
    TOO_DEEP.with(|f| f.set(false));
}

/// Whether the last parse gave up because the source nested too deeply.
pub fn nesting_overflowed() -> bool {
    TOO_DEEP.with(|f| f.get())
}

/// Where a declaration's body stopped making sense, and what kind of
/// declaration it was.
///
/// The parsers answer `Result<T, ()>`: a failure says nothing about why or
/// where. So a declaration whose HEADER parsed and whose body had one bad line
/// failed whole, `parse_top_level` tried every other parser on the same
/// keyword, and the error came out as ``unexpected `fun` `` pointing at the
/// declaration -- which is the one line that was fine.
///
/// Recording the failure at the body-item loop is enough to fix that, because
/// that loop is where a bad line stops the parse. The header is on the
/// declaration's own line, so a failure there already points where it should.
/// What `parse_top_level` could not get past.
pub struct ParseError {
    pub at: *const u8,
    /// The kind of declaration whose body it was, when the failure was inside
    /// one. `None` means nothing matched at the top level at all.
    pub inside: Option<&'static str>,
}

pub struct BodyFail {
    pub at: *const u8,
    pub kind: &'static str,
}

/// Keeps the furthest failure. Several parsers are tried on the same text and
/// only the one whose keyword matched gets into a body at all, but "furthest
/// wins" needs no coordination between them and does the right thing when a
/// construct nests.
fn note_body_fail(slot: &mut Option<BodyFail>, at: *const u8, kind: &'static str) {
    let is_further = match slot {
        None => true,
        Some(f) => at as usize > f.at as usize,
    };
    if is_further {
        *slot = Some(BodyFail { at, kind });
    }
}

/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_graph_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawGraphDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "graph ");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (graph_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (args, tail) = parse_arg_tuple(char_ptr, char_end_ptr)?;
    char_ptr = tail;

    let (body_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    if anchor_depth >= body_depth {
        return Err(());
    }

    let mut body = Vec::new();
    loop {
        let (stmt_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        if stmt_depth != body_depth {
            break;
        }
        char_ptr = tail;
        let (stmt, tail) = match try_parse_graph_stmt(char_ptr, char_end_ptr) {
            Ok(v) => v,
            Err(()) => {
                note_body_fail(fail, char_ptr, "graph");
                return Err(());
            }
        };
        if !line_is_finished(tail, char_end_ptr) {
            note_body_fail(fail, tail, "graph");
            return Err(());
        }
        body.push(stmt);
        char_ptr = tail;
    }

    Ok((RawGraphDecl { name: graph_name, args, body }, char_ptr))
}

/// One line of a graph body: a pipe declaration or an instantiation.
unsafe fn try_parse_graph_stmt(
    char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawGraphStmt, *const u8), ()> {
    if let Ok((pipe, tail)) = try_parse_graph_pipe(char_ptr, char_end_ptr) {
        return Ok((RawGraphStmt::Pipe(pipe), tail));
    }
    let (inst, tail) = try_parse_graph_instance(char_ptr, char_end_ptr)?;
    Ok((RawGraphStmt::Instance(inst), tail))
}

/// `let name: buffer T`.
///
/// No direction: an internal pipe has both ends inside the graph, and which
/// end is which is decided by the instances wired to it.
///
/// `let` rather than a keyword of its own. A graph body holds two kinds of
/// line and they are already distinguishable -- one ends in a type and the
/// other in an argument list -- so a third word would say nothing the parser
/// or the reader did not already have.
unsafe fn try_parse_graph_pipe(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawGraphPipe, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "let ");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ":");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;

    let (stream, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "stream ");
    let said = if stream {
        char_ptr = tail;
        PipeWord::Stream
    } else {
        let (is_buffer, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "buffer ");
        if is_buffer {
            char_ptr = tail;
            PipeWord::Buffer
        } else {
            PipeWord::Missing
        }
    };
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (type_expr, tail) = try_parse_type_expr(char_ptr, char_end_ptr)?;
    char_ptr = tail;

    Ok((RawGraphPipe { name, said, type_expr }, char_ptr))
}

/// `Name(a, b, c)`.
unsafe fn try_parse_graph_instance(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
) -> Result<(RawGraphInstance, *const u8), ()> {
    let (module, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "(");
    if !matched {
        return Err(());
    }
    char_ptr = tail;

    let mut args = Vec::new();
    // `Name()` is accepted here and rejected in lowering, where the arity it
    // should have had is known.
    let (_, tail) = skip_trivia(char_ptr, char_end_ptr);
    let (empty, tail) = strip_prefix_on_match(tail, char_end_ptr, ")");
    if empty {
        return Ok((RawGraphInstance { module, args }, tail));
    }
    loop {
        let (_, tail) = skip_trivia(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (arg, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
        args.push(arg);
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (is_comma, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ",");
        if is_comma {
            char_ptr = tail;
            continue;
        }
        let (is_rparen, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, ")");
        if is_rparen {
            char_ptr = tail;
            break;
        }
        return Err(());
    }
    Ok((RawGraphInstance { module, args }, char_ptr))
}

/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_process_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawProcessDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "process ");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (proc_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let outcome = parse_arg_tuple(char_ptr, char_end_ptr);
    let (args, tail) = match outcome {
        Ok(val) => val,
        Err(_) => return Err(()),
    };
    char_ptr = tail;
    let (body_ancore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = anchor_depth < body_ancore_depth;
    if !inbound {
        return Err(());
    } // no stmt process is invalid

    let mut stmts = Vec::new();
    loop {
        let (stmt_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = stmt_depth == body_ancore_depth;
        if !same_depth {
            // body ended
            break;
        }
        char_ptr = tail;
        let (inner_stmt, tail) =
            match parse_proc_inner_stmt(char_ptr, char_end_ptr, body_ancore_depth) {
                Ok(v) => v,
                Err(()) => {
                    note_body_fail(fail, char_ptr, "process");
                    return Err(());
                }
            };
        stmts.push(inner_stmt);
        char_ptr = tail;
    }
    let res = RawProcessDecl {
        name: proc_name,
        args,
        body: stmts,
    };
    Ok((res, char_ptr))
}

/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn try_parse_function_decl(
    mut char_ptr: *const u8,
    char_end_ptr: *const u8,
    anchor_depth: u32,
    fail: &mut Option<BodyFail>,
) -> Result<(RawFunctionDecl, *const u8), ()> {
    let (matched, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "fun ");
    if !matched {
        return Err(());
    }
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (func_name, tail) = try_parse_alphanum(char_ptr, char_end_ptr)?;
    char_ptr = tail;
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let outcome = parse_arg_tuple(char_ptr, char_end_ptr);
    let (args, tail) = match outcome {
        Ok(val) => val,
        Err(_) => return Err(()),
    };
    char_ptr = tail;

    // Optional return type
    let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
    char_ptr = tail;
    let (has_ret_type, tail) = strip_prefix_on_match(char_ptr, char_end_ptr, "->");
    let mut return_type = None;
    if has_ret_type {
        char_ptr = tail;
        let (_, tail) = skip_whitespaces(char_ptr, char_end_ptr);
        char_ptr = tail;
        let (ty, tail) = match try_parse_type_expr(char_ptr, char_end_ptr) {
            Ok(val) => val,
            Err(_) => return Err(()),
        };
        return_type = Some(ty);
        char_ptr = tail;
    }

    let (body_ancore_depth, _) = skip_trivia(char_ptr, char_end_ptr);
    let inbound = anchor_depth < body_ancore_depth;
    if !inbound {
        return Err(());
    }

    let mut stmts = Vec::new();
    loop {
        let (stmt_depth, tail) = skip_trivia(char_ptr, char_end_ptr);
        let same_depth = stmt_depth == body_ancore_depth;
        if !same_depth {
            // body ended
            break;
        }
        char_ptr = tail;
        let (inner_stmt, tail) =
            match parse_proc_inner_stmt(char_ptr, char_end_ptr, body_ancore_depth) {
                Ok(v) => v,
                Err(()) => {
                    note_body_fail(fail, char_ptr, "fun");
                    return Err(());
                }
            };
        stmts.push(inner_stmt);
        char_ptr = tail;
    }
    let res = RawFunctionDecl {
        name: func_name,
        args,
        return_type,
        body: stmts,
    };
    Ok((res, char_ptr))
}

/// # Safety
///
/// `char_ptr` and `char_end_ptr` must bracket one live buffer, and the
/// returned pointer is into that same buffer -- so it must not outlive the
/// `SourceMap` holding it. Every caller reaches this through
/// `driver::parse_source`, which owns that buffer for as long as the AST.
pub unsafe fn parse_top_level(
    char_ptr: *const u8,
    length: u32,
) -> Result<Vec<TopLevelDecl>, ParseError> {
    reset_nesting();
    let mut items = Vec::new();
    let mut char_ptr = char_ptr;
    let end = unsafe { char_ptr.add(length as usize) };

    // Where to blame if nothing matches. `char_ptr` is left sitting on the
    // newline before the next declaration by the block loops, so reporting it
    // would point at the end of the PREVIOUS declaration; the offending token
    // starts after the trivia.
    let mut stuck_at = char_ptr;
    // Where a declaration's body stopped making sense, if one of them got that
    // far. Reported in preference to `stuck_at`, which is the declaration
    // keyword and is the one place that was fine.
    let mut fail: Option<BodyFail> = None;

    loop {
        let (depth, tail) = skip_trivia(char_ptr, end);
        if tail == end {
            char_ptr = tail;
            break;
        }
        stuck_at = tail;

        if let Ok((proc_decl, tail)) = try_parse_process_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::ProcessStmt(proc_decl));
            continue;
        }

        if let Ok((func_decl, tail)) = try_parse_function_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::FunctionStmt(func_decl));
            continue;
        }

        if let Ok((func_decl, tail)) = try_parse_sequence_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::SequenceDecl(func_decl));
            continue;
        }

        // `tail`, not `char_ptr`. Probing the pre-trivia pointer meant a
        // struct could only ever parse as the very first item in a file --
        // after any other declaration, char_ptr sits on a newline.
        if let Ok((struct_decl, tail)) = try_parse_struct_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::StructDecl(struct_decl));
            continue;
        }

        if let Ok((enum_decl, tail)) = try_parse_enum_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::EnumDecl(enum_decl));
            continue;
        }

        if let Ok((graph_decl, tail)) = try_parse_graph_decl(tail, end, depth, &mut fail) {
            char_ptr = tail;
            items.push(TopLevelDecl::GraphDecl(graph_decl));
            continue;
        }

        break;
    }
    let consumed_whole_input = char_ptr == end;
    if !consumed_whole_input {
        // A body failure is always the better location: `stuck_at` is the
        // declaration keyword, which is the one line that parsed.
        if let Some(f) = fail {
            return Err(ParseError { at: f.at, inside: Some(f.kind) });
        }
        return Err(ParseError { at: stuck_at, inside: None });
    }
    Ok(items)
}

#[test]
fn enum_parsing_test() {
    let str = concat!(
        "enum IntOr\n",
        "  left(i1)\n",
        "  right(i1)\n",
    );

    let inp_str = str.as_bytes().as_ptr_range();
    let end_ptr = inp_str.end;
    let (depth, new_ptr) = skip_trivia(inp_str.start, end_ptr);
    let outcome = unsafe { try_parse_enum_decl(new_ptr, end_ptr, depth, &mut None) };

    match outcome {
        Ok((func, _ptr_)) => {
            println!("{:#?}", func);
        }
        Err(_) => panic!("Failed to parse enum"),
    }
}

#[test]
fn struct_parsing_test() {
    let str = concat!(
        "struct IntPair\n",
        "  fst: i1\n",
        "  snd: i1\n",
    );

    let inp_str = str.as_bytes().as_ptr_range();
    let end_ptr = inp_str.end;
    let (depth, new_ptr) = skip_trivia(inp_str.start, end_ptr);
    let outcome = unsafe { try_parse_struct_decl(new_ptr, end_ptr, depth, &mut None) };

    match outcome {
        Ok((func, _ptr_)) => {
            println!("{:#?}", func);
        }
        Err(_) => panic!("Failed to parse struct"),
    }
}

#[test]
fn function_parsing_test() {
    let str = concat!(
        "fun ok_ident(arg1: Ty, arg2: inout Ty2) -> RetTy\n",
        "  var x : [i1;1] = 1\n",
        "  loop\n",
        "    x += 1\n",
        "    break\n",
        "  return x\n"
    );

    let inp_str = str.as_bytes().as_ptr_range();
    let end_ptr = inp_str.end;
    let (depth, new_ptr) = skip_trivia(inp_str.start, end_ptr);
    let outcome = unsafe { try_parse_function_decl(new_ptr, end_ptr, depth, &mut None) };

    match outcome {
        Ok((func, _ptr_)) => {
            println!("{:#?}", func);
        }
        Err(_) => panic!("Failed to parse function"),
    }
}

#[test]
fn seqv_parsing_test() {
    let str = concat!(
        "sequence MyFunc(arg1: Ty, arg2: inout Ty2)\n",
        "  let x : [i1;0] = 1\n",
        "  |||\n",
        "  loop\n",
        "    break\n",
        "  return x\n"
    );
    println!("{}", str);

    let inp_str = str.as_bytes().as_ptr_range();
    let end_ptr = inp_str.end;
    let (depth, new_ptr) = skip_trivia(inp_str.start, end_ptr);
    let outcome = unsafe { try_parse_sequence_decl(new_ptr, end_ptr, depth, &mut None) };

    let (decl, _) = outcome.expect("sequence should parse");
    assert_eq!(crate::parse::anumspan_to_str(&decl.name), "MyFunc");
    // The `|||` stage separator must survive parsing. It is still discarded
    // downstream -- the pipeline scheduler is M4 -- but losing it here would
    // make that impossible to build.
    let separators = decl
        .body
        .iter()
        .filter(|s| matches!(s, RawSeqInnerStmt::SegmentSeparator))
        .count();
    assert_eq!(separators, 1, "the `|||` stage cut should be recorded");
}

#[test]
fn basic_stuff() {
    let str = concat!(
        "process Name (arg1: stream out Ty)\n",
        "   \n    \n",
        // "  let name: Ty = (x[a | b] << 132) * 172[1 + 2 .. a.b]\n",
        "  for i in 1..0\n",
        "   \n    \n",
        "    let _ = head(i)\n"
    );

    let inp_str = str.as_bytes().as_ptr_range();
    let end_ptr = inp_str.end;
    let (depth, new_ptr) = skip_trivia(inp_str.start, end_ptr);
    let outcome = unsafe { try_parse_process_decl(new_ptr, end_ptr, depth, &mut None) };

    // This used to print "naaah" on failure and pass regardless, so it was
    // not a test.
    let (decl, _) = outcome.expect("process should parse");
    assert_eq!(crate::parse::anumspan_to_str(&decl.name), "Name");
    assert!(matches!(
        decl.args.entries[0].qualifier,
        ArgTypeQualifier::StreamOut
    ));
    assert_eq!(decl.body.len(), 1, "the for-loop is the only statement");
    assert!(matches!(decl.body[0], InnerStmt::ForLoopStmt(_)));
}

#[test]
fn w3() {
    use crate::lex::parse_top_level;
    let str = concat!(
        "sequence Name (smth: stream in Ty)\n",
        "   \n    \n",
        // "  for i in 1..0\n",
        // "    let _ = \n",
        // "      break\n",
        // "    return\n",
        // "  expr\n",
        // "  let _ : [[i1;2];2] = expr\n",
        // "  |||\n",
        // "  let _ : [[i1;2];2] = expr\n",
        "  let _ = @try_pop(smth) .is_ok\n",
    );
    let inp_str = str.as_bytes().as_ptr_range();
    let start_ptr = inp_str.start;
    let end_ptr = inp_str.end;
    let span = (end_ptr as usize) - (start_ptr as usize);
    let smth = match unsafe { parse_top_level(start_ptr, span as u32) } {
        Ok(val) => val,
        Err(_) => panic!("sequence should parse"),
    };
    assert_eq!(smth.len(), 1);
    assert!(matches!(smth[0], TopLevelDecl::SequenceDecl(_)));
}
// ---- trivia tests --------------------------------------------------------
//
// These are assertion-based on purpose. The pre-existing tests in this file
// are `println!`-only and cannot fail, which is how the inverted arithmetic
// precedence in parse.rs survived.

#[cfg(test)]
fn parse_str(src: &str) -> Result<Vec<TopLevelDecl>, ()> {
    let range = src.as_bytes().as_ptr_range();
    let len = (range.end as usize) - (range.start as usize);
    unsafe { parse_top_level(range.start, len as u32) }.map_err(|_| ())
}

#[test]
fn line_comments_are_trivia() {
    // Every example in desc.md uses `--` comments; none of them parsed before.
    let src = concat!(
        "-- leading comment\n",
        "process Name (arg1: stream in i1) -- trailing comment\n",
        "  -- comment-only line, indented differently to the body\n",
        "        \n",
        "  let x = arg1 -- comment after code\n",
        "  return\n",
    );
    let decls = parse_str(src).expect("comments should be trivia");
    assert_eq!(decls.len(), 1);
    match &decls[0] {
        TopLevelDecl::ProcessStmt(p) => assert_eq!(p.body.len(), 2),
        other => panic!("expected a process, got {:?}", other),
    }
}

#[test]
fn crlf_parses_the_same_as_lf() {
    let lf = concat!(
        "process Name (arg1: stream in i1)\n",
        "  let x = arg1\n",
        "  return\n",
    );
    let crlf = lf.replace('\n', "\r\n");

    let a = parse_str(lf).expect("LF source should parse");
    let b = parse_str(&crlf).expect("CRLF source should parse");

    let body_len = |d: &TopLevelDecl| match d {
        TopLevelDecl::ProcessStmt(p) => p.body.len(),
        _ => panic!("expected a process"),
    };
    assert_eq!(a.len(), b.len());
    assert_eq!(body_len(&a[0]), body_len(&b[0]));
    assert_eq!(body_len(&a[0]), 2);
}

#[test]
fn trailing_spaces_at_eof_do_not_read_past_the_buffer() {
    // skip_trivia used to dereference without re-testing for the end here.
    // Under Miri this was UB; in release it read whatever followed the string.
    let src = "process Name (arg1: stream in i1)\n  return\n   ";
    let _ = parse_str(src);
}

#[test]
fn empty_source_does_not_read_past_the_buffer() {
    // strip_prefix_on_match dereferenced before its end check, so probing a
    // keyword at char_ptr == char_end_ptr read one byte past the buffer.
    let decls = parse_str("").expect("empty source is not an error");
    assert!(decls.is_empty());
}

#[test]
fn break_demands_a_delimiter() {
    let range = "breakfast".as_bytes().as_ptr_range();
    assert!(
        try_parse_break_stmt(range.start, range.end).is_err(),
        "`breakfast` must not lex as `break` followed by `fast`"
    );

    let range = "break\n".as_bytes().as_ptr_range();
    assert!(try_parse_break_stmt(range.start, range.end).is_ok());
}

#[test]
fn tabs_are_detected() {
    let src = "process Name (a: stream in i1)\n\treturn\n";
    let range = src.as_bytes().as_ptr_range();
    assert!(find_tab(range.start, range.end).is_some());

    let src = "process Name (a: stream in i1)\n  return\n";
    let range = src.as_bytes().as_ptr_range();
    assert!(find_tab(range.start, range.end).is_none());
}

#[test]
fn a_struct_parses_anywhere_not_only_first() {
    // The top-level loop probed structs with the pre-trivia pointer, which the
    // block loops deliberately leave sitting on a newline -- so a struct only
    // ever matched as the very first item in a file.
    let src = concat!(
        "struct First\n",
        "  a: i1\n",
        "process Name (p: stream in i1)\n",
        "  return\n",
        "struct Second\n",
        "  b: i1\n",
    );
    let decls = parse_str(src).expect("a struct after a process should parse");
    assert_eq!(decls.len(), 3, "got {:#?}", decls);
    assert!(matches!(decls[0], TopLevelDecl::StructDecl(_)));
    assert!(matches!(decls[1], TopLevelDecl::ProcessStmt(_)));
    assert!(matches!(decls[2], TopLevelDecl::StructDecl(_)));
}
