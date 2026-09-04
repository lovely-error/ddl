// `graph` lowered to structural Verilog.
//
// Everywhere else in this compiler, hierarchy is removed: a `fun` call is
// inlined, and one declaration becomes one flat module. A graph is the
// opposite and the only place instances come from. It computes nothing --
// there is no expression in a graph body -- so its module has no values, no
// registers and no memories. It has wires and instances.
//
// WHAT IT CHECKS, and why each one is worth checking here rather than leaving
// to the synthesizer:
//
//   * every pipe has exactly one producer and exactly one consumer. Verilog
//     would take two drivers on one net and resolve them to `x`; a pipe with
//     no producer would sit at `z` and read as an intermittent hang.
//   * both ends agree on the payload type. A width mismatch is a silent
//     truncation at an instance port, and it is silent in every tool.
//
// Cycles are fine and need no handling: a net is a net whichever order the
// instances appear in. desc.md:82 asks for that explicitly, and it is what
// makes a feedback path -- a retry queue, a credit return -- expressible.

use std::collections::BTreeMap;

use crate::diag::{Diag, DiagSink, SourceMap};
use crate::ir::{Instance, Module, Net, Port, PortDir};
use crate::lex::{AlphanumSpan, ArgTypeQualifier, PipeWord};
use crate::parse::{GraphDecl, GraphStmt, PrecArgDefTuple, anumspan_to_str};
use crate::symbols::Symbols;
use crate::ty::{Ty, resolve_type_expr};

/// One pipe parameter of something a graph can instantiate.
#[derive(Debug, Clone)]
pub struct PipeSig {
    pub name: String,
    pub is_input: bool,
    pub ty: Ty,
}

/// The pipe interface of a `process` or `sequence`, which is all a graph can
/// see of it. Constant parameters are folded inside the callee and are not
/// ports, so they do not appear here and are not connected.
#[derive(Debug, Clone)]
pub struct BlockSig {
    pub name: String,
    pub kind: &'static str,
    pub pipes: Vec<PipeSig>,
}

/// Reads the pipe interface off a declaration's parameter list.
///
/// Errors are not reported here: this runs over every process and sequence
/// before any of them is lowered, and a parameter that cannot be resolved will
/// be reported against the declaration itself when its turn comes. Reporting
/// twice, once without the context of the body, helps nobody.
pub fn signature_of(name: &str, kind: &'static str, args: &PrecArgDefTuple, syms: &Symbols) -> BlockSig {
    let mut pipes = Vec::new();
    for arg in &args.entries {
        let is_input = match arg.qualifier {
            ArgTypeQualifier::BufferIn => true,
            ArgTypeQualifier::BufferOut => false,
            // A constant parameter, or an error the callee will report. Both
            // are reported against the callee itself.
            _ => continue,
        };
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(_) => continue,
        };
        pipes.push(PipeSig { name: anumspan_to_str(&arg.arg_name).to_string(), is_input, ty });
    }
    BlockSig { name: name.to_string(), kind, pipes }
}

/// What an `extern` may declare.
///
/// Pipes only. A graph connects pipes and nothing else, so a parameter of any
/// other kind would be a port the graph has no way to reach -- silently
/// unconnected in the emitted instantiation, which is a wire left floating and
/// the sort of thing that shows up as a hang on real silicon.
pub fn check_extern(
    map: &crate::diag::SourceMap,
    decl: &crate::parse::ExternDecl,
    sink: &mut DiagSink,
) {
    for arg in &decl.args.entries {
        let is_pipe = matches!(
            arg.qualifier,
            ArgTypeQualifier::BufferIn | ArgTypeQualifier::BufferOut
        );
        if is_pipe {
            continue;
        }
        sink.push(
            Diag::error(
                map.span_of(&arg.arg_name),
                format!(
                    "`{}` is not a pipe, and a graph can connect nothing else",
                    anumspan_to_str(&arg.arg_name)
                ),
            )
            .with_note(
                "an `extern` declares the pipe interface of a module DDL did not compile; a constant, a `port` or a raw signal on one would be left unconnected",
            ),
        );
    }
}

/// One pipe inside the graph: a port of the graph, or a `pipe` declaration.
struct GraphPipeInfo {
    ty: Ty,
    /// Where it was declared -- the `let` line, or the parameter it is. A pipe
    /// nothing connects to has no instance to blame, and its declaration is
    /// the only place a reader can act on.
    declared_at: AlphanumSpan,
    /// A port of the enclosing graph rather than an internal wire. A graph
    /// input is produced from outside and a graph output consumed outside, so
    /// the endpoint that is missing inside the graph is not missing.
    external: Option<bool>,
    producers: Vec<AlphanumSpan>,
    consumers: Vec<AlphanumSpan>,
}

/// One combinator a graph asked for: which, how wide, and carrying what.
///
/// Collected rather than emitted here, because two graphs asking for the same
/// shape want one module instantiated twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombUse {
    pub kind: crate::ir_comb::Comb,
    pub fan: usize,
    pub ty: Ty,
}

pub fn lower_graph(
    map: &SourceMap,
    syms: &Symbols,
    sigs: &BTreeMap<String, BlockSig>,
    decl: &GraphDecl,
    combs: &mut Vec<CombUse>,
    sink: &mut DiagSink,
) -> Option<Module> {
    let graph_name = anumspan_to_str(&decl.name).to_string();
    let mut ports = Vec::new();
    let mut nets = Vec::new();
    let mut pipes: BTreeMap<String, GraphPipeInfo> = BTreeMap::new();

    // Clock and reset first, in the same positions a process puts them, so a
    // graph can itself be an instance in another graph.
    for implicit in ["clk", "rst_n"] {
        ports.push(Port { name: implicit.to_string(), dir: PortDir::In, ty: Ty::BOOL });
    }

    // ---- the graph's own ports ------------------------------------------
    for arg in &decl.args.entries {
        let name = anumspan_to_str(&arg.arg_name).to_string();
        if name == "clk" || name == "rst_n" {
            sink.err_at(&arg.arg_name, format!("`{}` is implicit on a graph", name));
            return None;
        }
        let is_input = match arg.qualifier {
            ArgTypeQualifier::BufferIn => true,
            ArgTypeQualifier::BufferOut => false,
            _ => {
                sink.push(
                    Diag::error(
                        map.span_of(&arg.arg_name),
                        format!("`{}` is not a pipe, and a graph connects nothing else", name),
                    )
                    .with_note("a graph parameter is `buffer in` or `buffer out`"),
                );
                return None;
            }
        };
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&arg.arg_name, e.message());
                return None;
            }
        };
        if pipes.contains_key(&name) {
            sink.err_at(&arg.arg_name, format!("`{}` is declared twice", name));
            return None;
        }

        let (vd, rd, dd) = if is_input {
            (PortDir::In, PortDir::Out, PortDir::In)
        } else {
            (PortDir::Out, PortDir::In, PortDir::Out)
        };
        ports.push(Port { name: format!("{}_wsalt", name), dir: vd, ty: crate::ir::SALT });
        ports.push(Port { name: format!("{}_rsalt", name), dir: rd, ty: crate::ir::SALT });
        let pair = Ty::Array(Box::new(ty.clone()), 2);
        ports.push(Port { name: format!("{}_data", name), dir: dd, ty: pair });

        pipes.insert(
            name,
            GraphPipeInfo {
                ty,
                declared_at: arg.arg_name,
                external: Some(is_input),
                producers: Vec::new(),
                consumers: Vec::new(),
            },
        );
    }

    // ---- internal pipes --------------------------------------------------
    for stmt in &decl.body {
        let pipe = match stmt {
            GraphStmt::Pipe(p) => p,
            GraphStmt::Instance(_) => continue,
        };
        let name = anumspan_to_str(&pipe.name).to_string();
        if pipes.contains_key(&name) {
            sink.err_at(&pipe.name, format!("`{}` is declared twice", name));
            return None;
        }
        let ty = match resolve_type_expr(&pipe.ty, syms) {
            Ok(t) => t,
            Err(e) => {
                sink.err_at(&pipe.name, e.message());
                return None;
            }
        };
        // `buffer` is written out rather than assumed. `let mid: u16` is the
        // shape of a mistake people will make now that the keyword is `let`,
        // and a pipe declaration that names no kind reads as a wire.
        match pipe.pipe_word {
            PipeWord::Buffer => {}
            PipeWord::Missing => {
                sink.push(
                    Diag::error(
                        map.span_of(&pipe.name),
                        format!("`{}` does not say what kind of pipe it is", name),
                    )
                    .with_note(
                        "write `let <name>: buffer <T>`, whose producer waits when both slots are full",
                    ),
                );
                return None;
            }
        }
        nets.push(Net { name: format!("{}_wsalt", name), ty: crate::ir::SALT });
        nets.push(Net { name: format!("{}_rsalt", name), ty: crate::ir::SALT });
        let pair = Ty::Array(Box::new(ty.clone()), 2);
        nets.push(Net { name: format!("{}_data", name), ty: pair });

        pipes.insert(
            name,
            GraphPipeInfo {
                ty,
                declared_at: pipe.name,
                external: None,
                producers: Vec::new(),
                consumers: Vec::new(),
            },
        );
    }

    // ---- instances -------------------------------------------------------
    let mut instances: Vec<Instance> = Vec::new();
    let mut used_names: BTreeMap<String, u32> = BTreeMap::new();

    for stmt in &decl.body {
        let inst = match stmt {
            GraphStmt::Instance(i) => i,
            GraphStmt::Pipe(_) => continue,
        };
        let module = anumspan_to_str(&inst.module).to_string();
        let sigs_for_this_instance: Option<&BlockSig>;
        if module == graph_name {
            sink.err_at(&inst.module, format!("`{}` cannot instantiate itself", module));
            return None;
        }
        // A combinator has no declaration to look up: its interface is
        // decided by the pipes it is given. The payload comes from the first
        // argument, and every other one has to agree -- which is the same
        // check a declared module gets, applied to a signature derived rather
        // than written.
        let synthesised;
        if let Some(kind) = crate::ir_comb::Comb::of(&module) {
            let needs_two_sides = inst.args.len() >= 2;
            if !needs_two_sides {
                sink.push(
                    Diag::error(
                        map.span_of(&inst.module),
                        format!("`{}` needs at least two pipes", module),
                    )
                    .with_note(match kind {
                        crate::ir_comb::Comb::Merge => {
                            "write `@merge(a, b, out)`: the sources, then the sink"
                        }
                        crate::ir_comb::Comb::Split => {
                            "write `@split(src, a, b)`: the source, then the sinks"
                        }
                    }),
                );
                return None;
            }
            let first = anumspan_to_str(&inst.args[0]).to_string();
            let ty = match pipes.get(&first) {
                Some(info) => info.ty.clone(),
                None => {
                    sink.err_at(
                        &inst.args[0],
                        format!("`{}` is not a pipe of this graph", first),
                    );
                    return None;
                }
            };
            let fan = kind.fan(inst.args.len());
            let use_ = CombUse { kind, fan, ty: ty.clone() };
            if !combs.contains(&use_) {
                combs.push(use_);
            }
            synthesised = BlockSig {
                name: crate::ir_comb::module_name(kind, fan, &ty),
                kind: "combinator",
                pipes: crate::ir_comb::pipe_names(kind, fan)
                    .into_iter()
                    .map(|(name, is_input)| PipeSig { name, is_input, ty: ty.clone() })
                    .collect(),
            };
            sigs_for_this_instance = Some(&synthesised);
        } else {
            sigs_for_this_instance = None;
        }

        let sig = match sigs_for_this_instance.or_else(|| sigs.get(&module)) {
            Some(s) => s,
            None => {
                let mut known: Vec<&str> = sigs.keys().map(|k| k.as_str()).collect();
                known.retain(|k| *k != graph_name);
                let note = if known.is_empty() {
                    "a graph instantiates a `process`, a `sequence` or another `graph`".to_string()
                } else {
                    format!("declared here: {}", known.join(", "))
                };
                sink.push(
                    Diag::error(
                        map.span_of(&inst.module),
                        format!("`{}` is not a process, sequence or graph", module),
                    )
                    .with_note(note),
                );
                return None;
            }
        };

        if inst.args.len() != sig.pipes.len() {
            sink.push(
                Diag::error(
                    map.span_of(&inst.module),
                    format!(
                        "`{}` has {} pipe parameter{}, but {} {} given",
                        module,
                        sig.pipes.len(),
                        if sig.pipes.len() == 1 { "" } else { "s" },
                        inst.args.len(),
                        if inst.args.len() == 1 { "was" } else { "were" }
                    ),
                )
                .with_note(format!(
                    "its pipes are: {}",
                    sig.pipes
                        .iter()
                        .map(|p| format!(
                            "{}: buffer {} {}",
                            p.name,
                            if p.is_input { "in" } else { "out" },
                            p.ty.display()
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
            return None;
        }

        // What gets INSTANTIATED. For everything the program declared that is
        // the name it was declared under; for a combinator it is the module
        // the compiler wrote, whose name says its shape -- `ddl_merge_2x32`
        // rather than the `@merge` the source spelled, which is not an
        // identifier a Verilog file could carry anyway.
        let module = sig.name.clone();

        // `mul3`, then `mul3_1`, `mul3_2` -- stable, and readable in a
        // waveform, which a bare `u0` is not.
        let seen = used_names.entry(module.clone()).or_insert(0);
        let inst_name = if *seen == 0 {
            format!("u_{}", module)
        } else {
            format!("u_{}_{}", module, seen)
        };
        *seen += 1;

        let mut conns = vec![
            ("clk".to_string(), "clk".to_string()),
            ("rst_n".to_string(), "rst_n".to_string()),
        ];
        let mut produces: Vec<String> = Vec::new();

        for (formal, actual) in sig.pipes.iter().zip(inst.args.iter()) {
            let actual_name = anumspan_to_str(actual).to_string();
            let info = match pipes.get_mut(&actual_name) {
                Some(i) => i,
                None => {
                    sink.push(
                        Diag::error(
                            map.span_of(actual),
                            format!("`{}` is not a pipe of this graph", actual_name),
                        )
                        .with_note("declare it with `let <name>: buffer <T>`, or make it a parameter"),
                    );
                    return None;
                }
            };

            if info.ty != formal.ty {
                sink.push(
                    Diag::error(
                        map.span_of(actual),
                        format!(
                            "`{}` carries `{}`, but `{}.{}` carries `{}`",
                            actual_name,
                            info.ty.display(),
                            module,
                            formal.name,
                            formal.ty.display()
                        ),
                    )
                    .with_note("a pipe and the port it connects to must carry the same type"),
                );
                return None;
            }
            // An instance whose port is an input CONSUMES the pipe.
            if formal.is_input {
                info.consumers.push(*actual);
            } else {
                info.producers.push(*actual);
                produces.push(actual_name.clone());
            }

            conns.push((format!("{}_wsalt", formal.name), format!("{}_wsalt", actual_name)));
            conns.push((format!("{}_rsalt", formal.name), format!("{}_rsalt", actual_name)));
            conns.push((format!("{}_data", formal.name), format!("{}_data", actual_name)));
        }

        instances.push(Instance { module, name: inst_name, conns, produces });
    }

    if instances.is_empty() {
        sink.push(
            Diag::error(
                map.span_of(&decl.name),
                format!("`{}` instantiates nothing", graph_name),
            )
            .with_note("a graph is its instances; an empty one would emit an unconnected module"),
        );
        return None;
    }

    if !check_endpoints(map, &graph_name, &pipes, sink) {
        return None;
    }

    Some(Module {
        name: graph_name,
        ports,
        values: Vec::new(),
        drivers: Vec::new(),
        regs: Vec::new(),
        mems: Vec::new(),
        asserts: Vec::new(),
        params: Vec::new(),
        nets,
        instances,
    })
}

/// Exactly one producer and exactly one consumer for every pipe.
///
/// desc.md:106 allows one producer and several consumers, by duplicating the
/// sink -- which is what `@split` does, and it is still not what naming a pipe
/// twice means. The duplication is real: each consumer gets its own pair of
/// entries and its own salt, so the pipe has somewhere to put the copies. Two
/// instances on one net have nowhere, and what comes out is two drivers on a
/// salt leg for the simulator to resolve to `x`.
fn check_endpoints(
    map: &SourceMap,
    graph_name: &str,
    pipes: &BTreeMap<String, GraphPipeInfo>,
    sink: &mut DiagSink,
) -> bool {
    let mut ok = true;
    for (name, info) in pipes {
        // A graph input arrives already produced; a graph output leaves to be
        // consumed outside. Either way the outside end is not missing.
        let (mut producers, mut consumers) = (info.producers.len(), info.consumers.len());
        match info.external {
            Some(true) => producers += 1,
            Some(false) => consumers += 1,
            None => {}
        }

        // An instance that named it, if any did; otherwise the declaration,
        // which is where a reader can act. Fabricating a span at the start of
        // the buffer -- which this did -- points at whatever declaration
        // happens to be first in the file.
        let where_to_blame = info
            .producers
            .iter()
            .chain(info.consumers.iter())
            .next()
            .copied()
            .unwrap_or(info.declared_at);
        let mut report = |msg: String, note: &str| {
            ok = false;
            sink.push(
                Diag::error(map.span_of(&where_to_blame), msg).with_note(note.to_string()),
            );
        };

        if producers == 0 {
            report(
                format!("nothing sends to `{}`", name),
                "every pipe needs a producer; an unconnected one reads as an intermittent hang",
            );
        } else if producers > 1 {
            report(
                format!("`{}` has {} producers", name, producers),
                "write `@merge(a, b, p)`, which grants one of them per cycle in rotation; \
                 two drivers on one net resolve to `x`",
            );
        }

        if consumers == 0 {
            report(
                format!("nothing receives from `{}` in `{}`", name, graph_name),
                "every pipe needs a consumer, or its producer stalls forever once the slot fills",
            );
        } else if consumers > 1 {
            report(
                format!("`{}` has {} consumers", name, consumers),
                "write `@split(p, a, b)`, which gives each consumer its own copy and its \
                 own pair of entries; naming one pipe twice puts two drivers on one salt",
            );
        }
    }
    ok
}
