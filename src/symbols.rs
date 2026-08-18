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
use crate::lex::ArgTypeQualifier;
use crate::parse::{EnumDecl, FunctionDecl, StructDecl, anumspan_to_str};
use crate::ty::{Ty, bits_for, const_eval};

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub width: u32,
    /// In declaration order, which is also the order a `match` must cover.
    pub variants: Vec<(String, u128)>,
}

impl EnumDef {
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
    /// `(name, is_output, ty)` in declaration order.
    pub params: Vec<(String, bool, Ty)>,
}

impl FuncSig {
    pub fn inputs(&self) -> impl Iterator<Item = &(String, bool, Ty)> {
        self.params.iter().filter(|(_, is_out, _)| !is_out)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &(String, bool, Ty)> {
        self.params.iter().filter(|(_, is_out, _)| *is_out)
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
/// Enums and structs are registered first so that a function signature may
/// name either, in any order.
pub fn build(
    enums: &[EnumDecl],
    structs: &[StructDecl],
    funcs: &[FunctionDecl],
    sink: &mut DiagSink,
) -> Symbols {
    let mut syms = Symbols::default();

    for decl in enums {
        if let Some(def) = build_enum(decl, sink) {
            register_enum(&mut syms, decl, def, sink);
        }
    }
    // A second pass, because a struct field may name an enum or another struct.
    for decl in structs {
        if let Some(def) = build_struct(decl, &syms, sink) {
            let name_is_taken =
                syms.structs.contains_key(&def.name) || syms.enums.contains_key(&def.name);
            if name_is_taken {
                sink.err_at(&decl.name, format!("`{}` is declared more than once", def.name));
                continue;
            }
            syms.structs.insert(def.name.clone(), def);
        }
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

fn register_enum(syms: &mut Symbols, decl: &EnumDecl, def: EnumDef, sink: &mut DiagSink) {
    let name_is_taken = syms.enums.contains_key(&def.name);
    if name_is_taken {
        sink.err_at(&decl.name, format!("`{}` is declared more than once", def.name));
        return;
    }
    for (variant, _) in &def.variants {
        let previous = syms.variant_owner.insert(variant.clone(), def.name.clone());
        if let Some(other) = previous {
            sink.err_at(
                &decl.name,
                format!("variant `{}` is already declared by `{}`", variant, other),
            );
        }
    }
    syms.enums.insert(def.name.clone(), def);
}

fn build_enum(decl: &EnumDecl, sink: &mut DiagSink) -> Option<EnumDef> {
    let name = anumspan_to_str(&decl.name).to_string();
    let mut variants: Vec<(String, u128)> = Vec::new();
    let mut next_discriminant: u128 = 0;

    for variant in &decl.variants {
        let vname = anumspan_to_str(&variant.name).to_string();
        if variant.payload.is_some() {
            sink.err_at(&variant.name, "enum variants cannot carry a payload yet");
            return None;
        }
        let discriminant = match &variant.discriminant {
            None => next_discriminant,
            Some(expr) => match const_eval(expr) {
                Ok(v) => v,
                Err(_) => {
                    sink.err_at(&variant.name, "discriminant must be a constant");
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

    Some(EnumDef { name, width, variants })
}

fn build_struct(decl: &StructDecl, syms: &Symbols, sink: &mut DiagSink) -> Option<StructDef> {
    let name = anumspan_to_str(&decl.name).to_string();
    let mut fields: Vec<(String, Ty)> = Vec::new();

    for field in &decl.fields {
        let fname = anumspan_to_str(&field.name).to_string();
        let is_duplicate = fields.iter().any(|(n, _)| *n == fname);
        if is_duplicate {
            sink.err_at(&field.name, format!("field `{}` is declared twice", fname));
            return None;
        }
        let ty = match crate::ty::resolve_type_expr(&field.field_type, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&field.name, e.message());
                return None;
            }
        };
        fields.push((fname, ty));
    }

    if fields.is_empty() {
        sink.err_at(&decl.name, "a struct needs at least one field");
        return None;
    }
    Some(StructDef { name, fields })
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
        let is_output = match arg.qualifier {
            ArgTypeQualifier::In => false,
            ArgTypeQualifier::Out => true,
            ArgTypeQualifier::Inout => {
                sink.err_at(&arg.arg_name, "`inout` parameters are not supported yet");
                return None;
            }
            _ => {
                sink.err_at(
                    &arg.arg_name,
                    "pipe parameters need a `process` or `sequence`, not a `fun`",
                );
                return None;
            }
        };
        params.push((pname, is_output, ty));
    }
    Some(FuncSig { name, params })
}
