// The global symbol table.
//
// Nothing used to populate one. `sema.rs` starts from an empty `HashSet`, so
// every reference to a user-defined type and every call to a user-defined
// function reported "unknown" -- which is why no DDL program with more than
// one declaration could ever have worked.
//
// Built once, after precedence resolution and before lowering, from the whole
// declaration list. That ordering is what makes declarations mutually visible:
// a `fun` may call one declared below it, and may name an enum declared
// anywhere in the file.

use std::collections::HashMap;

use crate::diag::{Diag, DiagSink};
use crate::lex::{AlphanumSpan, ArgTypeQualifier};
use crate::parse::{EnumDecl, FunctionDecl, StructDecl, anumspan_to_str};
use crate::ty::{Ty, bits_for, const_eval};

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    /// Tag plus payload. A variant's bits are `{tag, payload}` -- the tag in
    /// the HIGH bits, matching the way a struct puts its first field there, so
    /// the layout is the one a reader already knows.
    pub width: u32,
    /// Declared by `enum Name: uN` when no variant carries a payload, and
    /// derived from the largest discriminant when one does -- a tagged union
    /// may not be annotated, because the number would name the tag while
    /// reading as the width of the whole value.
    pub tag_width: u32,
    /// The widest payload any variant carries, and zero when none does. A
    /// variant with a narrower payload leaves the spare bits undefined; only
    /// the tag says which reading is the live one.
    pub payload_width: u32,
    /// In declaration order, which is also the order a `match` must cover.
    pub variants: Vec<(String, u128)>,
    /// Payload type by variant name, for the variants that carry one.
    pub payloads: HashMap<String, Ty>,
}

impl EnumDef {
    /// Whether any variant carries a payload, which is what decides whether a
    /// value of this enum is its own tag or has one inside it.
    pub fn is_tagged_union(&self) -> bool {
        self.payload_width > 0
    }

    /// The payload a variant carries, if it carries one.
    pub fn payload_of(&self, variant: &str) -> Option<&Ty> {
        self.payloads.get(variant)
    }

    /// The whole value a payload-free variant is: the tag, shifted up over the
    /// payload field it does not use.
    pub fn bare_value(&self, discriminant: u128) -> u128 {
        discriminant << self.payload_width
    }

    pub fn discriminant_of(&self, variant: &str) -> Option<u128> {
        self.variants
            .iter()
            .find(|(n, _)| n == variant)
            .map(|(_, d)| *d)
    }

    pub fn ty(&self) -> Ty {
        Ty::Enum { name: self.name.clone(), width: self.width }
    }
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    /// Declaration order. The FIRST field occupies the HIGH bits, matching
    /// SystemVerilog packed structs, so a DDL struct and its SV counterpart
    /// have the same bit layout and can cross a module boundary unchanged.
    pub fields: Vec<(String, Ty)>,
}

impl StructDef {
    pub fn bit_width(&self) -> u32 {
        self.fields.iter().map(|(_, t)| t.bit_width()).sum()
    }

    /// Inclusive `(hi, lo)` bit range of a field.
    pub fn field_range(&self, field: &str) -> Option<(u32, u32)> {
        let mut hi = self.bit_width();
        for (name, ty) in &self.fields {
            let lo = hi - ty.bit_width();
            if name == field {
                return Some((hi - 1, lo));
            }
            hi = lo;
        }
        None
    }

    pub fn field_ty(&self, field: &str) -> Option<Ty> {
        self.fields.iter().find(|(n, _)| n == field).map(|(_, t)| t.clone())
    }

    pub fn ty(&self) -> Ty {
        Ty::Struct { name: self.name.clone(), width: self.bit_width() }
    }
}

/// Everything the lowering needs to know about a function it is calling.
#[derive(Debug, Clone)]
pub struct FuncSig {
    pub name: String,
    /// `(name, direction, ty)` in declaration order.
    pub params: Vec<(String, ParamDir, Ty)>,
}

/// How a parameter passes, from desc.md:82-87.
///
/// `In` is by value. `Out` is by reference, write-only -- which is how a
/// function returns more than one thing. `InOut` is by reference and readable:
/// it takes an argument like an input AND updates the caller's variable like
/// an output, so it appears in both lists below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamDir {
    In,
    Out,
    InOut,
}

impl ParamDir {
    pub fn takes_an_argument(self) -> bool {
        matches!(self, ParamDir::In | ParamDir::InOut)
    }

    pub fn produces_a_value(self) -> bool {
        matches!(self, ParamDir::Out | ParamDir::InOut)
    }
}

impl FuncSig {
    /// Parameters that an argument is written for, in declaration order.
    pub fn inputs(&self) -> impl Iterator<Item = &(String, ParamDir, Ty)> {
        self.params.iter().filter(|(_, dir, _)| dir.takes_an_argument())
    }

    /// Parameters that carry a result back out.
    pub fn outputs(&self) -> impl Iterator<Item = &(String, ParamDir, Ty)> {
        self.params.iter().filter(|(_, dir, _)| dir.produces_a_value())
    }

    /// Just the `inout` ones, which are written back to the caller's variable.
    pub fn inouts(&self) -> impl Iterator<Item = &(String, ParamDir, Ty)> {
        self.params.iter().filter(|(_, dir, _)| *dir == ParamDir::InOut)
    }
}

#[derive(Debug, Default)]
pub struct Symbols {
    pub enums: HashMap<String, EnumDef>,
    pub structs: HashMap<String, StructDef>,
    pub funcs: HashMap<String, FuncSig>,
    /// Variant name to owning enum. Variants are visible unqualified, because
    /// that is how the generated k2g_pkg is written and used -- `LB_ADD`,
    /// never `lb_e::LB_ADD`.
    variant_owner: HashMap<String, String>,
}

impl Symbols {
    pub fn lookup_type(&self, name: &str) -> Option<Ty> {
        if let Some(e) = self.enums.get(name) {
            return Some(e.ty());
        }
        self.structs.get(name).map(|s| s.ty())
    }

    /// Resolves an unqualified variant name to its enum and discriminant.
    pub fn lookup_variant(&self, variant: &str) -> Option<(&EnumDef, u128)> {
        let owner = self.variant_owner.get(variant)?;
        let def = self.enums.get(owner)?;
        let discriminant = def.discriminant_of(variant)?;
        Some((def, discriminant))
    }
}

/// Collects every top-level declaration into one table.
///
/// Enums and structs resolve TOGETHER rather than one kind then the other,
/// because a payload may name a struct and a struct field may name an enum:
///
/// ```text
/// struct addr_t
///   page: u8
///   off:  u8
/// enum req_e
///   Read(addr_t)
/// ```
///
/// A fixed order cannot serve both directions. This walks a worklist instead,
/// finishing whatever can be finished and going round again until a pass makes
/// no progress. What is left after that is either a name nobody declared or a
/// cycle -- an enum whose payload contains itself has no finite width -- and
/// the two are told apart by whether the missing name is one of the
/// declarations still waiting.
pub fn build(
    enums: &[EnumDecl],
    structs: &[StructDecl],
    funcs: &[FunctionDecl],
    sink: &mut DiagSink,
) -> Symbols {
    let mut syms = Symbols::default();

    // Tags first. A discriminant is a constant and a tag width is either
    // written down or implied by the largest one, so neither waits on a type.
    let mut shells: Vec<Option<EnumShell>> = Vec::with_capacity(enums.len());
    for decl in enums {
        let shell = build_enum_shell(decl, sink);
        if let Some(shell) = &shell {
            register_variants(&mut syms, decl, shell, sink);
        }
        shells.push(shell);
    }

    // A declaration still waiting for a type it names.
    enum Pending {
        Enum(usize),
        Struct(usize),
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (ix, shell) in shells.iter().enumerate() {
        if shell.is_some() {
            pending.push(Pending::Enum(ix));
        }
    }
    for ix in 0..structs.len() {
        pending.push(Pending::Struct(ix));
    }

    loop {
        let mut progress = false;
        let mut still: Vec<Pending> = Vec::new();
        for item in pending {
            let finished = match &item {
                Pending::Enum(ix) => {
                    let shell = shells[*ix].as_ref().expect("only present shells are pending");
                    match finish_enum(&enums[*ix], shell, &syms) {
                        Ok(def) => {
                            register_named(&mut syms, &enums[*ix].name, Named::Enum(def), sink);
                            true
                        }
                        Err(_) => false,
                    }
                }
                Pending::Struct(ix) => match build_struct_quiet(&structs[*ix], &syms) {
                    Ok(def) => {
                        register_named(&mut syms, &structs[*ix].name, Named::Struct(def), sink);
                        true
                    }
                    Err(_) => false,
                },
            };
            if finished {
                progress = true;
            } else {
                still.push(item);
            }
        }
        pending = still;
        if pending.is_empty() || !progress {
            break;
        }
    }

    // A width annotation on a tagged union would say one thing and mean
    // another: `enum req_e: u2` reads as "two bits wide" and the value is 18,
    // because the payload sits under the tag. Reinterpreting the number as the
    // TAG width is worse still -- the same syntax would mean the whole value
    // for one enum and part of it for the next, and a struct field budgeted
    // from the declaration would be wrong by the width of the payload.
    //
    // So a tagged union is sized by the compiler and the annotation is
    // refused. Reported here rather than from the worklist, which retries and
    // would say it more than once, and after the enum is registered so that
    // everything naming it still resolves and this is the only complaint.
    for decl in enums {
        let name = anumspan_to_str(&decl.name);
        let def = match syms.enums.get(name) {
            Some(d) if d.is_tagged_union() => d,
            _ => continue,
        };
        if decl.tag_type.is_none() {
            continue;
        }
        let natural_tag = bits_for(def.variants.iter().map(|(_, d)| *d).max().unwrap_or(0));
        sink.push(
            Diag::error(
                sink.map().span_of(&decl.name),
                format!("`{}` carries a payload, so its width is not a choice", name),
            )
            .with_note(format!(
                "drop the `:` annotation -- {} variant(s) need a {}-bit tag and the widest payload is {} bits, so a value is {}",
                def.variants.len(),
                natural_tag,
                def.payload_width,
                natural_tag + def.payload_width
            )),
        );
    }

    // Whatever is left cannot be resolved. Report the error each one actually
    // hit, so a typo reads as a typo; a name that IS declared and still will
    // not resolve is a cycle, and says so.
    let stuck: Vec<String> = pending
        .iter()
        .map(|item| match item {
            Pending::Enum(ix) => anumspan_to_str(&enums[*ix].name).to_string(),
            Pending::Struct(ix) => anumspan_to_str(&structs[*ix].name).to_string(),
        })
        .collect();
    for item in &pending {
        let (at, err) = match item {
            Pending::Enum(ix) => {
                let shell = shells[*ix].as_ref().expect("only present shells are pending");
                (
                    enums[*ix].name.clone(),
                    finish_enum(&enums[*ix], shell, &syms).expect_err("it did not finish"),
                )
            }
            Pending::Struct(ix) => (
                structs[*ix].name.clone(),
                build_struct_quiet(&structs[*ix], &syms).expect_err("it did not finish"),
            ),
        };
        let (span, message, missing) = err;
        let names_a_stuck_declaration = missing.as_ref().is_some_and(|m| stuck.contains(m));
        // A name that IS declared and still will not resolve is a cycle, not a
        // typo, and "`s_t` is not a type" would send the reader looking for a
        // spelling mistake in a declaration that is right there.
        let diag = if names_a_stuck_declaration {
            let missing = missing.expect("checked");
            let mine = anumspan_to_str(&at).to_string();
            Diag::error(
                sink.map().span_of(&span),
                format!("`{}` cannot be sized", mine),
            )
            .with_note(format!(
                "it contains `{}`, which needs `{}`'s size to know its own -- one of them has to hold the other behind a tag or a fixed width",
                missing, mine
            ))
        } else {
            Diag::error(sink.map().span_of(&span), message)
        };
        sink.push(diag);
    }

    for decl in funcs {
        if let Some(sig) = build_func(decl, &syms, sink) {
            let name_is_taken = syms.funcs.contains_key(&sig.name);
            if name_is_taken {
                sink.err_at(&decl.name, format!("`{}` is declared more than once", sig.name));
                continue;
            }
            syms.funcs.insert(sig.name.clone(), sig);
        }
    }
    syms
}

/// An enum's tag, before its payloads are known.
struct EnumShell {
    name: String,
    variants: Vec<(String, u128)>,
    tag_width: u32,
}

enum Named {
    Enum(EnumDef),
    Struct(StructDef),
}

fn register_named(syms: &mut Symbols, at: &AlphanumSpan, def: Named, sink: &mut DiagSink) {
    let name = match &def {
        Named::Enum(e) => e.name.clone(),
        Named::Struct(s) => s.name.clone(),
    };
    let name_is_taken = syms.enums.contains_key(&name) || syms.structs.contains_key(&name);
    if name_is_taken {
        sink.err_at(at, format!("`{}` is declared more than once", name));
        return;
    }
    match def {
        Named::Enum(e) => {
            syms.enums.insert(name, e);
        }
        Named::Struct(s) => {
            syms.structs.insert(name, s);
        }
    }
}

/// Variant names are visible unqualified, and that does not wait on a payload.
fn register_variants(
    syms: &mut Symbols,
    decl: &EnumDecl,
    shell: &EnumShell,
    sink: &mut DiagSink,
) {
    for (variant, _) in &shell.variants {
        let previous = syms.variant_owner.insert(variant.clone(), shell.name.clone());
        if let Some(other) = previous {
            sink.err_at(
                &decl.name,
                format!("variant `{}` is already declared by `{}`", variant, other),
            );
        }
    }
}

/// What a declaration could not resolve: where to blame, what to say, and the
/// type name it was missing if it was missing one.
type ResolveFailure = (AlphanumSpan, String, Option<String>);

/// The payload types, and the width they add.
fn finish_enum(
    decl: &EnumDecl,
    shell: &EnumShell,
    syms: &Symbols,
) -> Result<EnumDef, ResolveFailure> {
    let mut payloads: HashMap<String, Ty> = HashMap::new();
    let mut payload_width = 0u32;

    for variant in &decl.variants {
        let payload = match variant.payload() {
            Some(p) => p,
            None => continue,
        };
        let ty = crate::ty::resolve_type_expr(payload, syms).map_err(|e| {
            (variant.name.clone(), e.message(), e.missing_type_name())
        })?;
        if ty.is_memory() {
            return Err((
                variant.name.clone(),
                "a memory cannot be an enum payload: it is storage rather than a value"
                    .to_string(),
                None,
            ));
        }
        payload_width = payload_width.max(ty.bit_width());
        payloads.insert(anumspan_to_str(&variant.name).to_string(), ty);
    }

    Ok(EnumDef {
        name: shell.name.clone(),
        width: shell.tag_width + payload_width,
        tag_width: shell.tag_width,
        payload_width,
        variants: shell.variants.clone(),
        payloads,
    })
}

fn build_struct_quiet(decl: &StructDecl, syms: &Symbols) -> Result<StructDef, ResolveFailure> {
    let name = anumspan_to_str(&decl.name).to_string();
    let mut fields: Vec<(String, Ty)> = Vec::new();

    for field in &decl.fields {
        let fname = anumspan_to_str(&field.name).to_string();
        let is_duplicate = fields.iter().any(|(n, _)| *n == fname);
        if is_duplicate {
            return Err((field.name.clone(), format!("field `{}` is declared twice", fname), None));
        }
        let ty = crate::ty::resolve_type_expr(&field.field_type, syms)
            .map_err(|e| (field.name.clone(), e.message(), e.missing_type_name()))?;
        fields.push((fname, ty));
    }

    if fields.is_empty() {
        return Err((decl.name.clone(), "a struct needs at least one field".to_string(), None));
    }
    Ok(StructDef { name, fields })
}

fn build_enum_shell(decl: &EnumDecl, sink: &mut DiagSink) -> Option<EnumShell> {
    let name = anumspan_to_str(&decl.name).to_string();
    let mut variants: Vec<(String, u128)> = Vec::new();
    let mut next_discriminant: u128 = 0;

    for variant in &decl.variants {
        let vname = anumspan_to_str(&variant.name).to_string();
        let discriminant = match variant.discriminant() {
            None => next_discriminant,
            Some(expr) => match const_eval(expr) {
                Ok(v) => v,
                Err(e) => {
                    sink.err_at(&variant.name, format!("discriminant {}", e.reason()));
                    return None;
                }
            },
        };
        // SystemVerilog rejects duplicate labels but happily accepts duplicate
        // VALUES, which is the dangerous direction -- the same trap
        // emu/src/sv_gen.rs validates against before emitting k2g_pkg.
        let value_already_used = variants.iter().any(|(_, d)| *d == discriminant);
        if value_already_used {
            sink.err_at(
                &variant.name,
                format!("discriminant {} is already used in `{}`", discriminant, name),
            );
            return None;
        }
        variants.push((vname, discriminant));
        next_discriminant = discriminant + 1;
    }

    if variants.is_empty() {
        sink.err_at(&decl.name, "an enum needs at least one variant");
        return None;
    }

    let largest = variants.iter().map(|(_, d)| *d).max().unwrap_or(0);
    let needed_width = bits_for(largest);
    let width = match &decl.tag_type {
        None => needed_width,
        Some(t) => {
            let declared = match crate::ty::resolve_type_expr(t, &Symbols::default()) {
                Ok(ty) => ty,
                Err(e) => {
                    sink.err_at(&decl.name, e.message());
                    return None;
                }
            };
            let declared_width = declared.bit_width();
            let holds_every_variant = declared_width >= needed_width;
            if !holds_every_variant {
                let span = sink.map().span_of(&decl.name);
                sink.push(
                    Diag::error(
                        span,
                        format!(
                            "`{}` is {} bits wide but its largest discriminant needs {}",
                            name, declared_width, needed_width
                        ),
                    )
                    .with_note("widen the tag, or lower the discriminant"),
                );
                return None;
            }
            declared_width
        }
    };

    Some(EnumShell { name, variants, tag_width: width })
}

fn build_func(decl: &FunctionDecl, syms: &Symbols, sink: &mut DiagSink) -> Option<FuncSig> {
    let name = anumspan_to_str(&decl.name).to_string();
    let mut params = Vec::new();

    for arg in &decl.args.entries {
        let pname = anumspan_to_str(&arg.arg_name).to_string();
        let ty = match crate::ty::resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        let dir = match arg.qualifier {
            ArgTypeQualifier::In => ParamDir::In,
            ArgTypeQualifier::Out => ParamDir::Out,
            ArgTypeQualifier::Inout => ParamDir::InOut,
            _ => {
                sink.err_at(
                    &arg.arg_name,
                    "pipe parameters need a `process` or `sequence`, not a `fun`",
                );
                return None;
            }
        };
        params.push((pname, dir, ty));
    }
    Some(FuncSig { name, params })
}
