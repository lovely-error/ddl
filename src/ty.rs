// Types and widths.
//
// An HDL is width-first: `8'h00` and `16'h0000` are different hardware, and
// every Verilog declaration needs a width. Before this module the compiler had
// no type representation at all -- `PrecTypeExpr` is a syntax node, and the
// only check was that an identifier started with the letter `i`, which
// accepted `item`, `input` and bare `i`.
//
// ---- the width discipline ------------------------------------------------
//
// Binary arithmetic and bitwise operators require their operands to have the
// SAME width and signedness. Mixing them is an error that names the fix.
//
// This is deliberately stricter than Verilog, where operands are silently
// extended to the width of the widest, and stricter than Chisel, where
// everything auto-widens. Silent width coercion is exactly the bug class an
// HDL type system exists to catch, and "the compiler quietly picked a width
// for you" is the musing this language is supposed to remove.
//
// Two escape valves keep it from being tiresome:
//
//   * Unsized literals adopt the width of the other operand, checked to fit.
//     So `x - 1` works for any width of `x`, and `x - 300` is an error when
//     `x` is `u8`.
//   * Comparisons never complain. They widen internally to hold both operands
//     exactly, including the extra bit a mixed-signedness pair needs. That is
//     the rule k2g_alu.sv:15 spells out by hand: "i32 spans [-2^31, 2^31-1]
//     and u32 spans [0, 2^32-1]; their union does not fit in 32 bits, so a
//     32-bit comparator must misorder some mixed-tag pairs."

use crate::parse::{BuiltinOp, Literal, PrecResExpr, PrecTypeExpr, anumspan_to_str};
use crate::symbols::Symbols;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    /// `uN` -- unsigned, N bits.
    UInt(u32),
    /// `iN` -- signed, two's complement, N bits.
    SInt(u32),
    /// `[T; n]`
    Array(Box<Ty>, u32),
    /// A named enum. Carries its width so the backend needs no lookup; the
    /// variant list lives in `Symbols`.
    Enum { name: String, width: u32 },
    /// A named packed struct, likewise carrying its total width.
    Struct { name: String, width: u32 },
    /// `#[impl(k)] [T; n]` -- storage, not a value.
    ///
    /// Distinct from `Array` because the two lower to different things: an
    /// `Array` is a packed vector that can be a port, a struct field or an
    /// operand, and a `Mem` is an unpacked array with a write port and a read
    /// port that only a subscript can reach.
    Mem { elem: Box<Ty>, len: u32, kind: MemKind },
}

/// Which FPGA resource the array asks to be built from. desc.md:128.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemKind {
    /// Distributed RAM: one synchronous write port, asynchronous reads.
    /// GowinSynthesis infers SSRAM from exactly that shape, measured at
    /// RAM16SDP4 x32 for the 32x32 K2G value array.
    LutRam,
    /// Block RAM: one synchronous write port, synchronous reads.
    BlockRam,
    /// Banked, with conflict minimisation.
    BankedRam,
}

impl MemKind {
    pub fn parse(name: &str) -> Option<MemKind> {
        match name {
            "lutram" => Some(MemKind::LutRam),
            "bram" => Some(MemKind::BlockRam),
            "bkram" => Some(MemKind::BankedRam),
            _ => None,
        }
    }

    pub fn display(&self) -> &'static str {
        match self {
            MemKind::LutRam => "lutram",
            MemKind::BlockRam => "bram",
            MemKind::BankedRam => "bkram",
        }
    }
}

impl Ty {
    pub const BOOL: Ty = Ty::UInt(1);

    /// Total bits, which is also the width of the packed Verilog vector this
    /// type lowers to.
    pub fn bit_width(&self) -> u32 {
        match self {
            Ty::UInt(w) | Ty::SInt(w) => *w,
            Ty::Array(elem, n) => elem.bit_width() * n,
            Ty::Enum { width, .. } | Ty::Struct { width, .. } => *width,
            // Total storage. Never the width of a wire -- a memory is not a
            // value and `is_memory` gates every place one could be used as
            // one -- but a definite number is more useful than a panic.
            Ty::Mem { elem, len, .. } => elem.bit_width() * len,
        }
    }

    /// Total bits, or `None` if the product does not fit a `u32`.
    ///
    /// `bit_width` multiplies without checking, which is right only because
    /// every type that exists has been through this on the way in. It is the
    /// construction sites that enforce it -- `resolve_type_expr` for arrays
    /// and memories, `build_struct_quiet` for the field sum -- so a `Ty` whose
    /// width does not fit can never be built. Before that, `[u65536; 65536]`
    /// panicked in debug at the multiply and wrapped to zero in release, which
    /// then produced a port declared `[4294967295:0]` from an internal width
    /// of nothing.
    pub fn checked_bit_width(&self) -> Option<u32> {
        match self {
            Ty::UInt(w) | Ty::SInt(w) => Some(*w),
            Ty::Array(elem, n) => elem.checked_bit_width()?.checked_mul(*n),
            Ty::Enum { width, .. } | Ty::Struct { width, .. } => Some(*width),
            Ty::Mem { elem, len, .. } => elem.checked_bit_width()?.checked_mul(*len),
        }
    }

    pub fn is_memory(&self) -> bool {
        matches!(self, Ty::Mem { .. })
    }

    /// Whether storage appears anywhere inside this type.
    ///
    /// `is_memory` asks about the top level only, which is the right question
    /// for "can this be used as a value here" and the wrong one for building
    /// an aggregate. A struct field or an array element that was a memory got
    /// past every check and then lost its meaning: `#[impl(bram)] [u8; 4]`
    /// inside a struct lowered to an ordinary 32-bit packed vector, with the
    /// storage annotation silently discarded.
    ///
    /// `Ty::Struct` carries only a width, not its fields -- which is sound
    /// here because a struct containing storage is refused at construction, so
    /// no such type can exist to be asked about.
    pub fn contains_memory(&self) -> bool {
        match self {
            Ty::Mem { .. } => true,
            Ty::Array(elem, _) => elem.contains_memory(),
            Ty::UInt(_) | Ty::SInt(_) | Ty::Enum { .. } | Ty::Struct { .. } => false,
        }
    }

    pub fn is_signed(&self) -> bool {
        matches!(self, Ty::SInt(_))
    }

    pub fn is_scalar(&self) -> bool {
        matches!(self, Ty::UInt(_) | Ty::SInt(_))
    }

    /// Same signedness, given width.
    pub fn with_width(&self, w: u32) -> Ty {
        if self.is_signed() { Ty::SInt(w) } else { Ty::UInt(w) }
    }

    pub fn display(&self) -> String {
        match self {
            Ty::UInt(w) => format!("u{}", w),
            Ty::SInt(w) => format!("i{}", w),
            Ty::Array(elem, n) => format!("[{}; {}]", elem.display(), n),
            Ty::Enum { name, .. } | Ty::Struct { name, .. } => name.clone(),
            Ty::Mem { elem, len, kind } => {
                format!("#[impl({})] [{}; {}]", kind.display(), elem.display(), len)
            }
        }
    }

    pub fn is_enum(&self) -> bool {
        matches!(self, Ty::Enum { .. })
    }
}

#[derive(Debug, Clone)]
pub enum TyError {
    /// Not a type name at all.
    UnknownType(String),
    /// `u0` / `i0`, or a width that does not fit in u32.
    BadWidth(String),
    /// Array length that would not fold, and what stopped it.
    BadArrayLen(ConstError),
    /// Folded, and not a length: zero, or past what an index could address.
    ArrayLenOutOfRange(u128),
    /// Arrays are parsed but not yet lowered.
    Unsupported(String),
    /// The packed width does not fit a `u32`, whatever the length was.
    WidthOutOfRange,
}

impl TyError {
    pub fn message(&self) -> String {
        match self {
            TyError::UnknownType(n) => format!(
                "`{}` is not a type; expected `uN` (unsigned), `iN` (signed) or `[T; n]`",
                n
            ),
            TyError::BadWidth(n) => format!("`{}` has an invalid width", n),
            TyError::BadArrayLen(e) => format!("array length {}", e.reason()),
            // Two ends, and they are different mistakes: nothing to hold, and
            // more than an index could reach. One message covering both would
            // be wrong at whichever end the reader was standing at.
            TyError::ArrayLenOutOfRange(0) => {
                "an array length of 0 holds nothing; it must be 1 or more".to_string()
            }
            TyError::ArrayLenOutOfRange(n) => format!(
                "array length {} is past what an index can address; the limit is {}",
                n,
                u32::MAX
            ),
            TyError::Unsupported(what) => format!("{} is not supported yet", what),
            TyError::WidthOutOfRange => format!(
                "this type is wider than {} bits, which is the widest the compiler can represent",
                u32::MAX
            ),
        }
    }

    /// The name this failed to find, when it failed to find one.
    ///
    /// The symbol table resolves enums and structs together and retries what
    /// does not resolve, so it has to tell "waiting for a declaration that has
    /// not been finished yet" apart from "waiting forever".
    pub fn missing_type_name(&self) -> Option<String> {
        match self {
            TyError::UnknownType(n) => Some(n.clone()),
            _ => None,
        }
    }
}

/// Parses `uN` / `iN`.
///
/// Replaces the old check, which accepted any identifier beginning with `i`
/// and never looked at the width.
pub fn parse_scalar_type_name(name: &str) -> Result<Ty, TyError> {
    let bytes = name.as_bytes();
    if bytes.len() < 2 {
        return Err(TyError::UnknownType(name.to_string()));
    }
    let signed = match bytes[0] {
        b'u' => false,
        b'i' => true,
        _ => return Err(TyError::UnknownType(name.to_string())),
    };
    let digits = &name[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TyError::UnknownType(name.to_string()));
    }
    let width: u32 = match digits.parse() {
        Ok(w) => w,
        Err(_) => return Err(TyError::BadWidth(name.to_string())),
    };
    if width == 0 {
        return Err(TyError::BadWidth(name.to_string()));
    }
    Ok(if signed { Ty::SInt(width) } else { Ty::UInt(width) })
}

pub fn resolve_type_expr(expr: &PrecTypeExpr, syms: &Symbols) -> Result<Ty, TyError> {
    match expr {
        PrecTypeExpr::Ident(span) => {
            let name = anumspan_to_str(span);
            // A user-declared name wins over the built-in spelling, so an
            // enum called `u8` would shadow the scalar type rather than being
            // silently ignored.
            if let Some(ty) = syms.lookup_type(name) {
                return Ok(ty);
            }
            parse_scalar_type_name(name)
        }
        PrecTypeExpr::Array(elem, len) => {
            let elem = resolve_type_expr(elem, syms)?;
            let len = const_eval(len).map_err(TyError::BadArrayLen)?;
            let len_is_usable = len > 0 && len <= u32::MAX as u128;
            if !len_is_usable {
                return Err(TyError::ArrayLenOutOfRange(len));
            }
            // An array of storage is not storage-shaped: the annotation names
            // one backing store, and the array would need one per element.
            if elem.contains_memory() {
                return Err(TyError::Unsupported("an array of memories".to_string()));
            }
            let ty = Ty::Array(Box::new(elem), len as u32);
            // A length that fits and an element that fits can still multiply
            // to something that does not.
            if ty.checked_bit_width().is_none() {
                return Err(TyError::WidthOutOfRange);
            }
            Ok(ty)
        }
        PrecTypeExpr::MemArray { elem, len, kind } => {
            let elem_ty = resolve_type_expr(elem, syms)?;
            // A memory of memories has no meaning: the annotation names one
            // backing store, and nesting would need two.
            if elem_ty.contains_memory() {
                return Err(TyError::Unsupported("a memory of memories".to_string()));
            }
            let len = const_eval(len).map_err(TyError::BadArrayLen)?;
            let len_is_usable = len > 0 && len <= u32::MAX as u128;
            if !len_is_usable {
                return Err(TyError::ArrayLenOutOfRange(len));
            }
            let ty = Ty::Mem { elem: Box::new(elem_ty), len: len as u32, kind: *kind };
            if ty.checked_bit_width().is_none() {
                return Err(TyError::WidthOutOfRange);
            }
            Ok(ty)
        }
    }
}

// ---- constant evaluation -------------------------------------------------

#[derive(Debug, Clone)]
pub enum ConstError {
    NotConstant,
    DivideByZero,
    Overflow,
}

impl ConstError {
    /// Completes a sentence naming what was being folded: "array length ...",
    /// "discriminant ...". The caller knows what it asked for and this knows
    /// what became of it, and neither knows the other half.
    ///
    /// Worth keeping apart because they are different mistakes. `[u32; n]` is
    /// waiting for a value the compiler does not have; `[u32; 8 / 0]` has every
    /// value it needs and no answer. Reporting the second as the first sends a
    /// reader looking for a runtime variable that is not there.
    pub fn reason(&self) -> &'static str {
        match self {
            ConstError::NotConstant => "must be a constant known at compile time",
            ConstError::DivideByZero => "divides by zero",
            ConstError::Overflow => "overflows a 128-bit constant",
        }
    }
}

/// Folds an expression to a compile-time constant.
///
/// Everything the backend needs as a width goes through here, because
/// GowinSynthesis exits 1 with an empty log on `$clog2` inside a part-select
/// and on width casts in expressions (docs/bring-up.md). Emitting a folded
/// literal is the only safe option, so folding must happen in the compiler.
pub fn const_eval(expr: &PrecResExpr) -> Result<u128, ConstError> {
    fold(expr).map(|f| f.value)
}

/// A folded constant together with the width it carries.
///
/// Two domains, and which one an expression is in comes from its literals.
///
/// `None` is the mathematical domain: an unsized literal has no width, so
/// `4 * 64` is 256 and running past `u128` is an error rather than a wrap.
/// This is what an array length or a loop bound is written in.
///
/// `Some(w)` is the hardware domain, where the answer is what a `w`-bit
/// register would hold. The IR computes sized arithmetic that way, and this
/// used not to: `a[8'd255 + 8'd1]` folded to 256 here and was rejected as an
/// index past the end, while `let i = 8'd255 + 8'd1` then `a[i]` lowered to an
/// eight-bit sum that wrapped to 0 and read element 0. Naming the expression
/// changed both whether it was accepted and what it meant.
#[derive(Clone, Copy)]
struct Folded {
    value: u128,
    /// The width of the sized literals this was folded from, if any.
    width: Option<u32>,
}

impl Folded {
    fn math(value: u128) -> Self {
        Folded { value, width: None }
    }

    /// A predicate is a `u1` whichever domain its operands came from.
    fn bool(b: bool) -> Self {
        Folded { value: b as u128, width: Some(1) }
    }

    /// Truncates to the carried width, the way a register would.
    fn wrapped(value: u128, width: Option<u32>) -> Self {
        let value = match width {
            Some(w) if w < 128 => value & ((1u128 << w) - 1),
            _ => value,
        };
        Folded { value, width }
    }
}

/// The width an operation's result carries.
///
/// Mixing two different widths is a width error, which lowering reports
/// against the operands with a span to point at. Folding has neither, so it
/// takes the wider of the two and lets the real check speak.
fn joined_width(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn fold(expr: &PrecResExpr) -> Result<Folded, ConstError> {
    match expr {
        PrecResExpr::Literal(Literal::IntLiteral { overflow: true, .. }) => {
            Err(ConstError::Overflow)
        }
        PrecResExpr::Literal(Literal::IntLiteral { value, width, .. }) => {
            Ok(Folded::wrapped(*value, *width))
        }
        PrecResExpr::Call { base, args } => {
            let op = match &**base {
                PrecResExpr::Builtin(op) => op,
                _ => return Err(ConstError::NotConstant),
            };
            if args.len() == 1 {
                let a = fold(&args[0])?;
                return match op {
                    BuiltinOp::Neg => Ok(Folded::wrapped(a.value.wrapping_neg(), a.width)),
                    BuiltinOp::BitInvert => Ok(Folded::wrapped(!a.value, a.width)),
                    BuiltinOp::LogNot => Ok(Folded::bool(a.value == 0)),
                    _ => Err(ConstError::NotConstant),
                };
            }
            if args.len() != 2 {
                return Err(ConstError::NotConstant);
            }
            let lhs = fold(&args[0])?;
            let rhs = fold(&args[1])?;
            let (a, b) = (lhs.value, rhs.value);
            let w = joined_width(lhs.width, rhs.width);
            // In the hardware domain the answer is what the register holds, so
            // running off the top is a wrap and not an error. In the
            // mathematical domain there is no top to run off, so it is.
            let sized = w.is_some();
            match op {
                BuiltinOp::Add if sized => Ok(Folded::wrapped(a.wrapping_add(b), w)),
                BuiltinOp::Sub if sized => Ok(Folded::wrapped(a.wrapping_sub(b), w)),
                BuiltinOp::Mul if sized => Ok(Folded::wrapped(a.wrapping_mul(b), w)),
                BuiltinOp::Add => a.checked_add(b).map(Folded::math).ok_or(ConstError::Overflow),
                BuiltinOp::Sub => a.checked_sub(b).map(Folded::math).ok_or(ConstError::Overflow),
                BuiltinOp::Mul => a.checked_mul(b).map(Folded::math).ok_or(ConstError::Overflow),
                // `checked_*` like the four around it, and not only for the
                // symmetry: `/` and `%` by zero panic in every profile, so the
                // guard is load-bearing, and on a signed type the hand-written
                // `b == 0` would still miss `MIN / -1`, which overflows and
                // panics too. This folds in `u128`, where that case does not
                // exist -- but the spelling that cannot be wrong costs nothing.
                BuiltinOp::Div => a
                    .checked_div(b)
                    .map(|v| Folded::wrapped(v, w))
                    .ok_or(ConstError::DivideByZero),
                BuiltinOp::Mod => a
                    .checked_rem(b)
                    .map(|v| Folded::wrapped(v, w))
                    .ok_or(ConstError::DivideByZero),
                BuiltinOp::Pow => {
                    let exp: u32 = b.try_into().map_err(|_| ConstError::Overflow)?;
                    match a.checked_pow(exp) {
                        Some(v) => Ok(Folded::wrapped(v, w)),
                        // A sized power wraps like the multiplications it is.
                        None if sized => {
                            let mut acc: u128 = 1;
                            for _ in 0..exp {
                                acc = acc.wrapping_mul(a);
                            }
                            Ok(Folded::wrapped(acc, w))
                        }
                        None => Err(ConstError::Overflow),
                    }
                }
                BuiltinOp::Shl => {
                    let v = if b >= 128 { 0 } else { a << b };
                    Ok(Folded::wrapped(v, w))
                }
                BuiltinOp::Shr => {
                    let v = if b >= 128 { 0 } else { a >> b };
                    Ok(Folded::wrapped(v, w))
                }
                BuiltinOp::And => Ok(Folded::wrapped(a & b, w)),
                BuiltinOp::Or => Ok(Folded::wrapped(a | b, w)),
                BuiltinOp::Xor => Ok(Folded::wrapped(a ^ b, w)),
                BuiltinOp::Eq => Ok(Folded::bool(a == b)),
                BuiltinOp::Ne => Ok(Folded::bool(a != b)),
                BuiltinOp::Lt => Ok(Folded::bool(a < b)),
                BuiltinOp::Gt => Ok(Folded::bool(a > b)),
                BuiltinOp::Le => Ok(Folded::bool(a <= b)),
                BuiltinOp::Ge => Ok(Folded::bool(a >= b)),
                BuiltinOp::LogAnd => Ok(Folded::bool(a != 0 && b != 0)),
                BuiltinOp::LogOr => Ok(Folded::bool(a != 0 || b != 0)),
                _ => Err(ConstError::NotConstant),
            }
        }
        _ => Err(ConstError::NotConstant),
    }
}

/// Smallest number of bits that can hold `value` as an unsigned number.
///
/// A free function rather than an emitted `$clog2`, for the same reason
/// `emu/src/hostproto.rs` has its own `len_bits`: an inline `$clog2` in a
/// part-select is one of the constructs GowinSynthesis rejects with an empty
/// log.
pub fn bits_for(value: u128) -> u32 {
    if value == 0 { 1 } else { 128 - value.leading_zeros() }
}

/// Does `value` fit in `ty` without losing information?
pub fn literal_fits(value: u128, ty: &Ty) -> bool {
    match ty {
        Ty::Enum { width, .. } | Ty::Struct { width, .. } => {
            let holds_every_value = *width >= 128;
            holds_every_value || value < (1u128 << width)
        }
        Ty::UInt(w) => {
            let holds_every_value = *w >= 128;
            holds_every_value || value < (1u128 << w)
        }
        Ty::SInt(w) => {
            // Accept both a plain magnitude and an already-two's-complement
            // bit pattern of the right width.
            let holds_every_value = *w >= 128;
            let fits_as_bit_pattern = !holds_every_value && value < (1u128 << w);
            holds_every_value || fits_as_bit_pattern
        }
        Ty::Array(..) | Ty::Mem { .. } => false,
    }
}

// ---- operator typing -----------------------------------------------------

#[derive(Debug, Clone)]
pub enum OpTyError {
    WidthMismatch { lhs: Ty, rhs: Ty },
    SignednessMismatch { lhs: Ty, rhs: Ty },
    NotScalar(Ty),
    NeedsBool(Ty),
    LiteralTooWide { value: u128, ty: Ty },
}

impl OpTyError {
    pub fn message(&self) -> String {
        match self {
            OpTyError::WidthMismatch { lhs, rhs } => format!(
                "width mismatch: `{}` and `{}`",
                lhs.display(),
                rhs.display()
            ),
            OpTyError::SignednessMismatch { lhs, rhs } => format!(
                "signedness mismatch: `{}` and `{}`",
                lhs.display(),
                rhs.display()
            ),
            OpTyError::NotScalar(t) => {
                format!("expected an integer, found `{}`", t.display())
            }
            OpTyError::NeedsBool(t) => format!(
                "logical operators need `u1`, found `{}`",
                t.display()
            ),
            OpTyError::LiteralTooWide { value, ty } => {
                format!("literal {} does not fit in `{}`", value, ty.display())
            }
        }
    }

    /// The "here is what to write instead" half of the diagnostic.
    pub fn note(&self) -> Option<String> {
        match self {
            OpTyError::WidthMismatch { lhs, rhs } => {
                let (narrow, wide) = if lhs.bit_width() < rhs.bit_width() {
                    (lhs, rhs)
                } else {
                    (rhs, lhs)
                };
                let cast = if narrow.is_signed() { "@sext" } else { "@zext" };
                Some(format!(
                    "widen with `{}(x, {})`, or narrow with `@trunc(x, {})`",
                    cast,
                    wide.bit_width(),
                    narrow.bit_width()
                ))
            }
            OpTyError::SignednessMismatch { .. } => {
                Some("convert with `@signed(x)` or `@unsigned(x)`".to_string())
            }
            _ => None,
        }
    }
}

/// Result type of a binary operator, given already-typed operands.
///
/// `lhs`/`rhs` must already have had unsized literals adapted by the caller;
/// see `adapt_literal`.
pub fn binop_result(op: BuiltinOp, lhs: &Ty, rhs: &Ty) -> Result<Ty, OpTyError> {
    use BuiltinOp::*;

    // Enums compare but do not do arithmetic: `op == ARITH_SUB` is the point
    // of having them, while `op + 1` is a category error.
    let is_comparison = matches!(op, Eq | Ne | Lt | Gt | Le | Ge);
    if is_comparison {
        let both_are_the_same_enum = lhs == rhs && lhs.is_enum();
        if both_are_the_same_enum {
            return Ok(Ty::BOOL);
        }
    }

    if !lhs.is_scalar() {
        return Err(OpTyError::NotScalar(lhs.clone()));
    }
    if !rhs.is_scalar() {
        return Err(OpTyError::NotScalar(rhs.clone()));
    }

    match op {
        // Comparisons never complain about widths: they widen internally to
        // hold both operands exactly, plus one bit when signedness is mixed.
        Eq | Ne | Lt | Gt | Le | Ge => Ok(Ty::BOOL),

        LogAnd | LogOr => {
            if *lhs != Ty::BOOL {
                return Err(OpTyError::NeedsBool(lhs.clone()));
            }
            if *rhs != Ty::BOOL {
                return Err(OpTyError::NeedsBool(rhs.clone()));
            }
            Ok(Ty::BOOL)
        }

        // The shift amount is independent of the value's width, so no match
        // is required. The result keeps the left operand's type, which means
        // bits shifted off the top are lost -- widen first if you need them.
        // `>>` is arithmetic when the left operand is signed, logical when it
        // is unsigned, which is how k2g_shift.sv picks between SHR and SHRA.
        Shl | Shr => Ok(lhs.clone()),

        // The one operator that widens. A full-width product is always what
        // was meant, and Verilog's truncation to the operand width here is a
        // classic source of silently wrong hardware.
        Mul => {
            let product_is_signed = lhs.is_signed() || rhs.is_signed();
            let product_width = lhs.bit_width() + rhs.bit_width();
            Ok(if product_is_signed {
                Ty::SInt(product_width)
            } else {
                Ty::UInt(product_width)
            })
        }

        Add | Sub | Div | Mod | And | Or | Xor => {
            let signedness_agrees = lhs.is_signed() == rhs.is_signed();
            if !signedness_agrees {
                return Err(OpTyError::SignednessMismatch {
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                });
            }
            let widths_agree = lhs.bit_width() == rhs.bit_width();
            if !widths_agree {
                return Err(OpTyError::WidthMismatch {
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                });
            }
            Ok(lhs.clone())
        }

        // Folded before it reaches here.
        Pow => Ok(lhs.clone()),

        _ => Err(OpTyError::NotScalar(lhs.clone())),
    }
}

/// The width a comparison must be performed at so that it cannot misorder.
///
/// Mixed signedness needs one bit more than the wider operand, because the
/// union of the two ranges does not fit in either. Returns the common type.
pub fn comparison_operand_ty(lhs: &Ty, rhs: &Ty) -> Ty {
    let signedness_is_mixed = lhs.is_signed() != rhs.is_signed();
    let extra_bit = if signedness_is_mixed { 1 } else { 0 };
    let common_width = lhs.bit_width().max(rhs.bit_width()) + extra_bit;

    let either_is_signed = lhs.is_signed() || rhs.is_signed();
    if either_is_signed {
        Ty::SInt(common_width)
    } else {
        Ty::UInt(common_width)
    }
}

/// Can a value of `from` be used where `to` is expected, with no explicit cast?
///
/// Only exact matches. Widening is not implicit either: in hardware, an
/// implicit extension is a decision about sign that the reader cannot see.
pub fn assignable(from: &Ty, to: &Ty) -> bool {
    from == to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_and_struct_widths() {
        let e = Ty::Enum { name: "arith_e".into(), width: 2 };
        assert_eq!(e.bit_width(), 2);
        assert_eq!(e.display(), "arith_e");
        assert!(!e.is_scalar(), "an enum is not an arithmetic type");
    }

    #[test]
    fn scalar_type_names() {
        assert_eq!(parse_scalar_type_name("u1").unwrap(), Ty::UInt(1));
        assert_eq!(parse_scalar_type_name("u32").unwrap(), Ty::UInt(32));
        assert_eq!(parse_scalar_type_name("i32").unwrap(), Ty::SInt(32));
    }

    #[test]
    fn identifiers_starting_with_i_are_not_types() {
        // The old check was `if let [b'i', tail @ ..]` with an is_alphanumeric
        // fold, so all of these typechecked as integers.
        for bad in ["item", "input", "i", "iFoo", "uFoo", "u0", "i0", "s32", "x32", ""] {
            assert!(
                parse_scalar_type_name(bad).is_err(),
                "`{}` must not parse as a type",
                bad
            );
        }
    }

    #[test]
    fn bit_widths() {
        assert_eq!(Ty::UInt(32).bit_width(), 32);
        assert_eq!(Ty::Array(Box::new(Ty::UInt(8)), 4).bit_width(), 32);
    }

    #[test]
    fn same_width_arithmetic_is_accepted() {
        let u32 = Ty::UInt(32);
        assert_eq!(binop_result(BuiltinOp::Add, &u32, &u32).unwrap(), u32);
        assert_eq!(binop_result(BuiltinOp::Xor, &u32, &u32).unwrap(), u32);
    }

    #[test]
    fn mixed_width_arithmetic_is_rejected_with_a_fix() {
        let err = binop_result(BuiltinOp::Add, &Ty::UInt(32), &Ty::UInt(5)).unwrap_err();
        assert!(err.message().contains("width mismatch"), "{}", err.message());
        let note = err.note().unwrap();
        assert!(note.contains("@zext(x, 32)"), "{}", note);
    }

    #[test]
    fn mixed_signedness_is_rejected() {
        let err = binop_result(BuiltinOp::Add, &Ty::UInt(32), &Ty::SInt(32)).unwrap_err();
        assert!(err.message().contains("signedness"), "{}", err.message());
    }

    #[test]
    fn multiplication_widens_to_the_full_product() {
        let r = binop_result(BuiltinOp::Mul, &Ty::UInt(32), &Ty::UInt(32)).unwrap();
        assert_eq!(r, Ty::UInt(64));
        let r = binop_result(BuiltinOp::Mul, &Ty::SInt(8), &Ty::UInt(4)).unwrap();
        assert_eq!(r, Ty::SInt(12));
    }

    #[test]
    fn shifts_keep_the_left_operand_type() {
        let r = binop_result(BuiltinOp::Shl, &Ty::UInt(32), &Ty::UInt(5)).unwrap();
        assert_eq!(r, Ty::UInt(32));
        // Arithmetic vs logical right shift is decided by the left operand.
        let r = binop_result(BuiltinOp::Shr, &Ty::SInt(32), &Ty::UInt(5)).unwrap();
        assert_eq!(r, Ty::SInt(32));
    }

    #[test]
    fn comparisons_yield_i1_and_never_complain() {
        let r = binop_result(BuiltinOp::Lt, &Ty::UInt(32), &Ty::SInt(8)).unwrap();
        assert_eq!(r, Ty::BOOL);
    }

    #[test]
    fn mixed_sign_comparison_gets_the_extra_bit() {
        // k2g_alu.sv:15: i32 and u32 together do not fit in 32 bits.
        let t = comparison_operand_ty(&Ty::SInt(32), &Ty::UInt(32));
        assert_eq!(t, Ty::SInt(33));
        // Same signedness needs no extra bit.
        let t = comparison_operand_ty(&Ty::UInt(32), &Ty::UInt(16));
        assert_eq!(t, Ty::UInt(32));
    }

    #[test]
    fn logical_operators_demand_i1() {
        assert!(binop_result(BuiltinOp::LogAnd, &Ty::BOOL, &Ty::BOOL).is_ok());
        let err = binop_result(BuiltinOp::LogAnd, &Ty::UInt(32), &Ty::BOOL).unwrap_err();
        assert!(err.message().contains("u1"), "{}", err.message());
    }

    #[test]
    fn literals_fit_checks() {
        assert!(literal_fits(255, &Ty::UInt(8)));
        assert!(!literal_fits(256, &Ty::UInt(8)));
        assert!(literal_fits(0, &Ty::UInt(1)));
        assert!(!literal_fits(2, &Ty::UInt(1)));
    }

    #[test]
    fn bits_for_matches_clog2_intent() {
        assert_eq!(bits_for(0), 1);
        assert_eq!(bits_for(1), 1);
        assert_eq!(bits_for(2), 2);
        assert_eq!(bits_for(255), 8);
        assert_eq!(bits_for(256), 9);
    }
}
