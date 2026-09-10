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
// instances appear in. desc.md:111 asks for that explicitly, and it is what
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

/// One `wire` parameter -- a bare signal of the declared width, carrying no
/// handshake at all.
///
/// Only an `extern` and a `graph` have these. They are the two declarations
/// that compute nothing, which is the whole reason a wire is safe there: a
/// body would have to wait on something, and a wire gives it nothing to wait
/// on. desc.md:29 is the line -- "compute only logic in ddl, io in verilog".
#[derive(Debug, Clone)]
pub struct WireSig {
    pub name: String,
    pub is_input: bool,
    pub ty: Ty,
}

/// One parameter, in the position it was declared in.
///
/// Ordered rather than split into two lists, because a graph passes arguments
/// POSITIONALLY and the two kinds interleave: `ext(pipe, pin, pipe)` has to
/// bind its second argument to the wire and not to the second pipe.
#[derive(Debug, Clone)]
pub enum ArgSig {
    Pipe(PipeSig),
    Wire(WireSig),
}

impl ArgSig {
    pub fn name(&self) -> &str {
        match self {
            ArgSig::Pipe(p) => &p.name,
            ArgSig::Wire(w) => &w.name,
        }
    }

    /// How the parameter reads back in a diagnostic, in source spelling.
    pub fn display(&self) -> String {
        match self {
            ArgSig::Pipe(p) => format!(
                "{}: buffer {} {}",
                p.name,
                if p.is_input { "in" } else { "out" },
                p.ty.display()
            ),
            ArgSig::Wire(w) => format!(
                "{}: wire {} {}",
                w.name,
                if w.is_input { "in" } else { "out" },
                w.ty.display()
            ),
        }
    }
}

/// The connectable interface of a declaration, which is all a graph can see of
/// it. Constant parameters are folded inside the callee and are not ports, so
/// they do not appear here and are not connected.
#[derive(Debug, Clone)]
pub struct BlockSig {
    pub name: String,
    pub kind: &'static str,
    pub args: Vec<ArgSig>,
}

impl BlockSig {
    /// The pipe parameters alone, for the callers that only ever had those.
    pub fn pipes(&self) -> impl Iterator<Item = &PipeSig> {
        self.args.iter().filter_map(|a| match a {
            ArgSig::Pipe(p) => Some(p),
            ArgSig::Wire(_) => None,
        })
    }
}

/// Reads the connectable interface off a declaration's parameter list.
///
/// Errors are not reported here: this runs over every process and sequence
/// before any of them is lowered, and a parameter that cannot be resolved will
/// be reported against the declaration itself when its turn comes. Reporting
/// twice, once without the context of the body, helps nobody.
pub fn signature_of(name: &str, kind: &'static str, args: &PrecArgDefTuple, syms: &Symbols) -> BlockSig {
    let mut sig_args = Vec::new();
    for arg in &args.entries {
        let is_wire = match arg.qualifier {
            ArgTypeQualifier::BufferIn | ArgTypeQualifier::BufferOut => false,
            ArgTypeQualifier::WireIn | ArgTypeQualifier::WireOut => true,
            // A constant parameter, or an error the callee will report. Both
            // are reported against the callee itself.
            _ => continue,
        };
        let is_input = matches!(
            arg.qualifier,
            ArgTypeQualifier::BufferIn | ArgTypeQualifier::WireIn
        );
        let ty = match resolve_type_expr(&arg.type_expr, syms) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let arg_name = anumspan_to_str(&arg.arg_name).to_string();
        sig_args.push(if is_wire {
            ArgSig::Wire(WireSig { name: arg_name, is_input, ty })
        } else {
            ArgSig::Pipe(PipeSig { name: arg_name, is_input, ty })
        });
    }
    BlockSig { name: name.to_string(), kind, args: sig_args }
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
    syms: &Symbols,
    sink: &mut DiagSink,
) {
    let mut names = std::collections::HashSet::new();
    for arg in &decl.args.entries {
        if !names.insert(anumspan_to_str(&arg.arg_name)) {
            sink.err_at(
                &arg.arg_name,
                "an external interface cannot declare the same name twice",
            );
        }
        // An `extern` is the one declaration with no body, so nothing else
        // ever looks at its parameter types: `signature_of` skips the ones it
        // cannot resolve on the understanding that whoever lowers the
        // declaration will report them, and an `extern` is never lowered. A
        // misspelled type therefore vanished from the interface and the
        // emitted instantiation left that port unconnected -- a floating input
        // on real hardware, from a typo, with exit code 0.
        if let Err(e) = resolve_type_expr(&arg.type_expr, syms) {
            sink.err_at(&arg.arg_name, e.message());
        }
        let is_connectable = matches!(
            arg.qualifier,
            ArgTypeQualifier::BufferIn
                | ArgTypeQualifier::BufferOut
                | ArgTypeQualifier::WireIn
                | ArgTypeQualifier::WireOut
        );
        if is_connectable {
            continue;
        }
        sink.push(
            Diag::error(
                map.span_of(&arg.arg_name),
                format!(
                    "`{}` is not a pipe or a wire, and a graph can connect nothing else",
                    anumspan_to_str(&arg.arg_name)
                ),
            )
            .with_note(
                "an `extern` declares the interface of a module DDL did not compile: `buffer in`/`buffer out` for a handshake, `wire in`/`wire out` for a bare signal",
            ),
        );
    }
}

/// Reports a graph hierarchy that contains itself.
///
/// A graph lowering its own body catches `a` instantiating `a`, because that
/// is the one cycle visible from inside a single declaration. It cannot see
/// `a` instantiating `b` and `b` instantiating `a`, and that pair compiled: it
/// produced two Verilog modules instantiating one another, which is not a
/// finite piece of hardware. The compiler itself does not hang -- the export
/// walk is finite -- so nothing complained.
///
/// Structural elaboration has no base case to stop at. There is no `if` around
/// an instance and no recursion depth to bottom out, so a cycle in the
/// instantiation edges is unbuildable however deep it is, and the whole cycle
/// is named rather than just the edge that closed it.
///
/// Feedback through a CHANNEL inside a finite hierarchy is a different thing
/// and stays legal. Only instantiation edges are walked here.
pub fn check_graph_cycles(
    map: &crate::diag::SourceMap,
    graphs: &[crate::parse::GraphDecl],
    sink: &mut DiagSink,
) {
    use std::collections::BTreeMap;

    let mut edges: BTreeMap<&str, Vec<&AlphanumSpan>> = BTreeMap::new();
    for g in graphs {
        let from = anumspan_to_str(&g.name);
        let out = edges.entry(from).or_default();
        for stmt in &g.body {
            if let crate::parse::GraphStmt::Instance(i) = stmt {
                out.push(&i.module);
            }
        }
    }

    // Grey means "on the current path", black means "explored and clean".
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Grey,
        Black,
    }
    let mut marks: BTreeMap<&str, Mark> = BTreeMap::new();
    let mut reported: std::collections::BTreeSet<&str> = Default::default();

    // An explicit stack rather than recursion: the depth here is the user's
    // graph nesting, and blowing the compiler's stack on a deep one would be
    // its own bug report.
    struct Frame<'a> {
        name: &'a str,
        next: usize,
    }

    for g in graphs {
        let root = anumspan_to_str(&g.name);
        if marks.contains_key(root) {
            continue;
        }
        let mut stack = vec![Frame { name: root, next: 0 }];
        marks.insert(root, Mark::Grey);
        while let Some(top) = stack.last_mut() {
            let name = top.name;
            let ix = top.next;
            top.next += 1;
            let Some(callee) = edges.get(name).and_then(|v| v.get(ix)) else {
                marks.insert(name, Mark::Black);
                stack.pop();
                continue;
            };
            let callee_name = anumspan_to_str(callee);
            // Only graphs can close a cycle: everything else is a leaf.
            if !edges.contains_key(callee_name) {
                continue;
            }
            match marks.get(callee_name) {
                Some(Mark::Black) => {}
                Some(Mark::Grey) => {
                    // Report once per cycle, against the edge that closes it,
                    // naming the whole path so the reader can see the loop.
                    if reported.insert(callee_name) {
                        let start = stack
                            .iter()
                            .position(|f| f.name == callee_name)
                            .unwrap_or(0);
                        let mut path: Vec<&str> =
                            stack[start..].iter().map(|f| f.name).collect();
                        path.push(callee_name);
                        // A one-node cycle reads better as what it is.
                        let msg = if path.len() <= 2 {
                            format!("`{}` cannot instantiate itself", callee_name)
                        } else {
                            format!(
                                "`{}` instantiates itself through {}",
                                callee_name,
                                path.join(" -> ")
                            )
                        };
                        sink.push(
                            Diag::error(map.span_of(callee), msg)
                            .with_note(
                                "a graph is elaborated structurally, so a hierarchy that contains itself has no finite hardware; feedback between instances belongs on a pipe, not on an instantiation",
                            ),
                        );
                    }
                }
                None => {
                    marks.insert(callee_name, Mark::Grey);
                    stack.push(Frame { name: callee_name, next: 0 });
                }
            }
        }
    }
}

/// One `wire` parameter of the graph, and who drives it.
///
/// A wire has no protocol to check, so the only thing worth counting is
/// drivers: two instances driving one `wire out` is not a race the salt
/// protocol resolves, it is two gates fighting over a net and an `x` in
/// simulation. An input may fan out to as many readers as like it.
struct GraphWireInfo {
    ty: Ty,
    declared_at: AlphanumSpan,
    /// A graph input is driven from outside, an output from within.
    is_input: bool,
    drivers: Vec<AlphanumSpan>,
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
#[derive(Debug, Clone)]
pub struct CombUse {
    pub kind: crate::ir_comb::Comb,
    pub fan: usize,
    pub ty: Ty,
}

/// The shape, which is the width and not the type.
///
/// The same rule as `AdaptUse`, for the same reason: a combinator routes its
/// payload and never looks inside it, so `ir_comb::module_name` names one by
/// `bit_width` and the body follows. Deriving this compared the whole `Ty`,
/// which made `@merge(a, b, o)` over `u16` and over a two-`u8` struct two
/// uses that both built `ddl_merge_2x16`.
impl PartialEq for CombUse {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.fan == other.fan
            && self.ty.bit_width() == other.ty.bit_width()
    }
}

impl Eq for CombUse {}

pub fn lower_graph(
    map: &SourceMap,
    syms: &Symbols,
    sigs: &BTreeMap<String, BlockSig>,
    decl: &GraphDecl,
    combs: &mut Vec<CombUse>,
    adapts: &mut Vec<crate::ir_adapt::AdaptUse>,
    flags: &crate::ir_export::ExportFlags,
    cdcs: &mut Vec<crate::ir_cdc_lib::CdcUse>,
    sink: &mut DiagSink,
) -> Option<Module> {
    let graph_name = anumspan_to_str(&decl.name).to_string();
    let mut ports = Vec::new();
    // One clock/reset pair per DOMAIN, however many pipes name it.
    let mut domains: Vec<String> = Vec::new();
    let mut nets = Vec::new();
    let mut pipes: BTreeMap<String, GraphPipeInfo> = BTreeMap::new();
    let mut wires: BTreeMap<String, GraphWireInfo> = BTreeMap::new();

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
        let (is_input, is_wire) = match arg.qualifier {
            ArgTypeQualifier::BufferIn => (true, false),
            ArgTypeQualifier::BufferOut => (false, false),
            ArgTypeQualifier::WireIn => (true, true),
            ArgTypeQualifier::WireOut => (false, true),
            _ => {
                sink.push(
                    Diag::error(
                        map.span_of(&arg.arg_name),
                        format!("`{}` is not a pipe or a wire, and a graph connects nothing else", name),
                    )
                    .with_note("a graph parameter is `buffer in`, `buffer out`, `wire in` or `wire out`"),
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
        if pipes.contains_key(&name) || wires.contains_key(&name) {
            sink.err_at(&arg.arg_name, format!("`{}` is declared twice", name));
            return None;
        }

        // A wire is one port of the declared width and nothing else. There is
        // no salt to carry and no enable beside it: the far side of a wire is
        // a pin or an IP block, and neither can be told to wait.
        if is_wire {
            ports.push(Port {
                name: name.clone(),
                dir: if is_input { PortDir::In } else { PortDir::Out },
                ty: ty.clone(),
            });
            wires.insert(
                name,
                GraphWireInfo { ty, declared_at: arg.arg_name.clone(), is_input, drivers: Vec::new() },
            );
            continue;
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
                declared_at: arg.arg_name.clone(),
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
                declared_at: pipe.name.clone(),
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
                args: crate::ir_comb::pipe_names(kind, fan)
                    .into_iter()
                    .map(|(name, is_input)| ArgSig::Pipe(PipeSig { name, is_input, ty: ty.clone() }))
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

        if inst.args.len() != sig.args.len() {
            sink.push(
                Diag::error(
                    map.span_of(&inst.module),
                    format!(
                        "`{}` has {} parameter{}, but {} {} given",
                        module,
                        sig.args.len(),
                        if sig.args.len() == 1 { "" } else { "s" },
                        inst.args.len(),
                        if inst.args.len() == 1 { "was" } else { "were" }
                    ),
                )
                .with_note(format!(
                    "it takes: {}",
                    sig.args
                        .iter()
                        .map(|a| a.display())
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
        let is_extern = sig.kind == "extern";

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
        // Emitted ahead of the instance they serve, so the file still reads
        // top to bottom: everything a connection names appears above it.
        let mut adapters: Vec<Instance> = Vec::new();

        for (formal, actual) in sig.args.iter().zip(inst.args.iter()) {
            let actual_name = anumspan_to_str(actual).to_string();

            let formal = match formal {
                ArgSig::Wire(w) => {
                    let info = match wires.get_mut(&actual_name) {
                        Some(i) => i,
                        None => {
                            sink.push(
                                Diag::error(
                                    map.span_of(actual),
                                    format!("`{}` is not a wire of this graph", actual_name),
                                )
                                .with_note(
                                    "a wire reaches a graph only as a parameter; declare it with `<name>: wire in <T>` or `wire out <T>`",
                                ),
                            );
                            return None;
                        }
                    };
                    if info.ty != w.ty {
                        sink.push(
                            Diag::error(
                                map.span_of(actual),
                                format!(
                                    "`{}` carries `{}`, but `{}.{}` carries `{}`",
                                    actual_name,
                                    info.ty.display(),
                                    module,
                                    w.name,
                                    w.ty.display()
                                ),
                            )
                            .with_note("a wire and the port it connects to must be the same width"),
                        );
                        return None;
                    }
                    // An instance whose wire port is an OUTPUT drives the net.
                    // Driving a graph input means two sources on one wire, one
                    // of them off-chip, which no protocol arbitrates.
                    if !w.is_input {
                        if info.is_input {
                            sink.push(
                                Diag::error(
                                    map.span_of(actual),
                                    format!("`{}` is an input of this graph, and `{}.{}` drives it", actual_name, module, w.name),
                                )
                                .with_note("an input wire is driven from outside; declare it `wire out` to drive it here"),
                            );
                            return None;
                        }
                        info.drivers.push(actual.clone());
                    }
                    conns.push((w.name.clone(), actual_name));
                    continue;
                }
                ArgSig::Pipe(p) => p,
            };

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
                info.consumers.push(actual.clone());
            } else {
                info.producers.push(actual.clone());
                produces.push(actual_name.clone());
            }

            // A module DDL compiled speaks salt, because both ends of the
            // wire were written by the same compiler. A module it did not
            // gets a FIFO, and the translation is a module the compiler
            // writes -- so the hand-written file never sees a salt.
            if !is_extern {
                conns.push((format!("{}_wsalt", formal.name), format!("{}_wsalt", actual_name)));
                conns.push((format!("{}_rsalt", formal.name), format!("{}_rsalt", actual_name)));
                conns.push((format!("{}_data", formal.name), format!("{}_data", actual_name)));
                continue;
            }

            let kind = crate::ir_adapt::Adapt::at(formal.is_input, true);
            let use_ = crate::ir_adapt::AdaptUse { kind, ty: formal.ty.clone() };
            if !adapts.contains(&use_) {
                adapts.push(use_);
            }

            // Three nets between the adapter and the extern, named for the
            // instance and the port so two instances of one extern do not
            // collide.
            let [flag, go, data] = kind.face();
            let leg = |suffix: &str| format!("{}_{}_{}", inst_name, formal.name, suffix);
            nets.push(Net { name: leg(flag), ty: Ty::BOOL });
            nets.push(Net { name: leg(go), ty: Ty::BOOL });
            nets.push(Net { name: leg(data), ty: formal.ty.clone() });

            let salt = kind.salt_pipe();
            let mut adapt_conns = vec![
                ("clk".to_string(), "clk".to_string()),
                ("rst_n".to_string(), "rst_n".to_string()),
                (format!("{}_wsalt", salt), format!("{}_wsalt", actual_name)),
                (format!("{}_rsalt", salt), format!("{}_rsalt", actual_name)),
                (format!("{}_data", salt), format!("{}_data", actual_name)),
            ];

            // Is this pipe of this instance asked to cross a clock domain?
            let crossed = flags.crossings.iter().find(|c| {
                c.owner == graph_name
                    && c.instance.as_deref() == Some(inst_name.as_str())
                    && c.pipe == formal.name
            });

            if let Some(c) = crossed {
                // The extern keeps its `clk`/`rst_n` -- that is its core clock,
                // and nothing here touches it. What the crossed pipe adds is
                // `<pipe>_clk` and `<pipe>_rst_n` on the SAME `<pipe>_<suffix>`
                // ABI the face already uses, so the compiler never has to guess
                // a clock name or make the author declare a `wire`.
                let width = formal.ty.bit_width();
                let ck = crate::ir_cdc_lib::Cdc::at(formal.is_input, true);
                let use_ = crate::ir_cdc_lib::CdcUse { kind: ck, width, depth: c.depth };
                if !cdcs.contains(&use_) {
                    cdcs.push(use_);
                }

                let clk_port = format!("{}_clk", c.domain);
                let rst_port = format!("{}_rst_n", c.domain);
                if !domains.contains(&c.domain) {
                    domains.push(c.domain.clone());
                    ports.push(Port { name: clk_port.clone(), dir: PortDir::In, ty: Ty::BOOL });
                    ports.push(Port { name: rst_port.clone(), dir: PortDir::In, ty: Ty::BOOL });
                }

                let mut shell = vec![
                    ("clk".to_string(), "clk".to_string()),
                    ("rst_n".to_string(), "rst_n".to_string()),
                    ("f_clk".to_string(), clk_port),
                ];
                for (ix, suffix) in [flag, go, data].iter().enumerate() {
                    // The adapter's face lands on internal nets; the shell
                    // carries it the rest of the way to the extern.
                    let core_net = format!("{}_{}_c_{}", inst_name, formal.name, suffix);
                    nets.push(Net {
                        name: core_net.clone(),
                        ty: if ix == 2 { formal.ty.clone() } else { Ty::BOOL },
                    });
                    adapt_conns.push((suffix.to_string(), core_net.clone()));
                    shell.push((format!("c_{}", suffix), core_net));
                    shell.push((format!("f_{}", suffix), leg(suffix)));
                    conns.push((format!("{}_{}", formal.name, suffix), leg(suffix)));
                }
                // The face's own clock and reset, named the way the face is.
                conns.push((format!("{}_clk", formal.name), format!("{}_clk", c.domain)));
                conns.push((format!("{}_rst_n", formal.name), rst_port));

                adapters.push(Instance {
                    module: crate::ir_cdc_lib::module_name(ck, width, c.depth),
                    name: format!("{}_{}_cdc", inst_name, formal.name),
                    conns: shell,
                    produces: Vec::new(),
                });
            } else {
                for suffix in [flag, go, data] {
                    adapt_conns.push((suffix.to_string(), leg(suffix)));
                    conns.push((format!("{}_{}", formal.name, suffix), leg(suffix)));
                }
            }
            // The adapter drives no pipe of its own as far as the picture is
            // concerned: the extern is what the source named, and blaming the
            // adapter for a connection nobody wrote would be a worse drawing.
            adapters.push(Instance {
                module: crate::ir_adapt::module_name(kind, &formal.ty),
                name: format!("{}_{}_adapt", inst_name, formal.name),
                conns: adapt_conns,
                produces: Vec::new(),
            });
        }

        instances.extend(adapters);
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

    if !check_wires(map, &wires, sink) {
        return None;
    }

    if !check_endpoints(map, &graph_name, &pipes, sink) {
        return None;
    }

    Some(Module {
        calls: Vec::new(),
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
/// desc.md:135 allows one producer and several consumers, by duplicating the
/// sink -- which is what `@split` does, and it is still not what naming a pipe
/// twice means. The duplication is real: each consumer gets its own pair of
/// entries and its own salt, so the pipe has somewhere to put the copies. Two
/// instances on one net have nowhere, and what comes out is two drivers on a
/// salt leg for the simulator to resolve to `x`.
/// Every `wire out` of the graph is driven exactly once.
///
/// A wire carries no protocol, so this is the whole of what can be checked
/// about one -- and it is worth checking, because the two failures are silent.
/// Two drivers is an `x` the simulator resolves and the synthesizer may not;
/// none is a floating output, which on real hardware is a pin that reads as
/// whatever the board leaks into it.
fn check_wires(
    map: &SourceMap,
    wires: &BTreeMap<String, GraphWireInfo>,
    sink: &mut DiagSink,
) -> bool {
    let mut ok = true;
    for (name, info) in wires {
        if info.is_input {
            continue;
        }
        match info.drivers.len() {
            1 => {}
            0 => {
                sink.push(
                    Diag::error(
                        map.span_of(&info.declared_at),
                        format!("`{}` is an output wire that nothing drives", name),
                    )
                    .with_note("connect it to a `wire out` parameter of something this graph instantiates"),
                );
                ok = false;
            }
            n => {
                sink.push(
                    Diag::error(
                        map.span_of(&info.drivers[1]),
                        format!("`{}` is driven by {} instances", name, n),
                    )
                    .with_note("a wire has no arbitration; exactly one thing may drive it"),
                );
                ok = false;
            }
        }
    }
    ok
}

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
            .cloned()
            .unwrap_or(info.declared_at.clone());
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
