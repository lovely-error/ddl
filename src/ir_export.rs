// Export targets: which module an invocation is FOR, and what its boundary
// looks like.
//
// An export target is the module a synthesizer or a hand-written file is meant
// to instantiate: the module the build is about, and the one whose ports a
// person has to wire up by hand.
//
// It decides two things, and they are not the same. WHAT IS EMITTED is the
// targets and everything they use, transitively -- a graph cannot instantiate
// a sequence that is not in the file. Nothing else goes in: a module unrelated
// to what was asked for is one a synthesizer would elaborate, a linter would
// check, and a reader would have to account for. WHAT IS WRAPPED is narrower
// still: only the targets themselves present a FIFO, because everything else
// in the file is internal to one of them.
//
// So a target does not present the salt protocol. It presents a FIFO, and the
// translation is a wrapper this file writes: the lowered module keeps its
// logic and takes the name `<name>_core`, and a new module under the original
// name holds the FIFO ports, one adapter per pipe, and one instance of the
// core. Nothing about how anything lowers changes -- a wrapper is ports, nets
// and instances with no values, which is exactly the shape a `graph` already
// has.
//
// WHICH module is decided from the use graph. A root is a candidate that
// nothing else uses, where "uses" means both halves: the instances a `graph`
// builds, and the calls a body inlined. Only the second is easy to get wrong,
// because an inlined call leaves no `Instance` behind -- `k2g_alu` calls
// `rdt_is_signed` and `Module.instances` has never heard of it.
//
// And a candidate comes only from a file named on the command line. An
// `import` brings in dependencies, not deliverables: three of the examples
// carry `uop_nop` and `uop_fault` from `k2g_types.ddl` and call neither, and
// counting those as roots would turn three unambiguous files into a question.
//
// When one root survives all that, it is the target and no flag is needed.
// When several do, the compiler asks rather than guessing, because guessing
// picks what the file is for on the author's behalf.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::Diag;
use crate::ir::{Instance, Module, Net, Port, PortDir, SALT};
use crate::ir_adapt::{Adapt, AdaptUse};
use crate::ir_graph::{ArgSig, BlockSig};
use crate::ty::Ty;

/// The suffix a wrapped module's logic takes, so the wrapper can have the name.
pub const CORE_SUFFIX: &str = "_core";

/// What the flags asked for.
#[derive(Debug, Default, Clone)]
pub struct ExportFlags {
    /// `--export a,b` -- these present the FIFO interface.
    pub export: Vec<String>,
    /// `--bare-export a,b` -- these keep the raw salt ports.
    pub bare: Vec<String>,
}

impl ExportFlags {
    pub fn is_empty(&self) -> bool {
        self.export.is_empty() && self.bare.is_empty()
    }
}

/// Every module that nothing else uses, among the ones offered as candidates.
///
/// Both halves of the use graph: what a `graph` instantiates, and what a body
/// inlined. Neither alone is the answer.
pub fn roots(modules: &[Module], candidates: &BTreeSet<String>) -> Vec<String> {
    let mut used: BTreeSet<&str> = BTreeSet::new();
    for module in modules {
        for inst in &module.instances {
            used.insert(inst.module.as_str());
        }
        for callee in &module.calls {
            used.insert(callee.as_str());
        }
    }
    modules
        .iter()
        .filter(|m| candidates.contains(&m.name) && !used.contains(m.name.as_str()))
        .map(|m| m.name.clone())
        .collect()
}

/// What an invocation emits, and which of it presents a FIFO.
///
/// Two different sets, and conflating them is what made a file carry modules
/// that had nothing to do with what was asked for.
#[derive(Debug, Default, Clone)]
pub struct Exports {
    /// The modules the build is for. These, and everything they use, are
    /// emitted; nothing else is.
    pub keep: Vec<String>,
    /// Of those, the ones presenting the FIFO interface. The rest keep the
    /// salt ports -- either because they are internal to something here, or
    /// because `--bare-export` asked for them as they lower.
    pub wrap: Vec<String>,
}

/// Everything reachable from `seeds`, following both halves of the use graph.
///
/// A file is the targets and their dependencies. Emitting more is not a
/// kindness: an unrelated module in the file is one a synthesizer will elaborate,
/// a linter will check, and a reader will wonder about.
pub fn closure(modules: &[Module], seeds: &[String]) -> BTreeSet<String> {
    let by_name: BTreeMap<&str, &Module> =
        modules.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = seeds.to_vec();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let Some(m) = by_name.get(name.as_str()) else {
            // An `extern` is named by an instance and has no module here.
            continue;
        };
        for inst in &m.instances {
            queue.push(inst.module.clone());
        }
        for callee in &m.calls {
            queue.push(callee.clone());
        }
    }
    seen
}

/// Decides what to emit and what to wrap.
///
/// Explicit flags win. With no flag and one root, that root. With no flag and
/// several, an error that names them: which module the file is for is the
/// author's to say, and picking one silently is how a build ends up shipping
/// the wrong top level.
pub fn resolve(
    modules: &[Module],
    candidates: &BTreeSet<String>,
    flags: &ExportFlags,
) -> Result<Exports, Diag> {
    let known: BTreeSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    let listed = || {
        let mut names: Vec<&str> = known.iter().copied().collect();
        names.sort();
        names
            .iter()
            .map(|n| format!("`{}`", n))
            .collect::<Vec<_>>()
            .join(", ")
    };
    for name in flags.export.iter().chain(flags.bare.iter()) {
        if !known.contains(name.as_str()) {
            return Err(Diag::error_no_span(format!(
                "no module named `{}` in this compilation",
                name
            ))
            .with_note(format!("it emits: {}", listed())));
        }
    }
    for name in &flags.export {
        if flags.bare.contains(name) {
            return Err(Diag::error_no_span(format!(
                "`{}` is given to both --export and --bare-export",
                name
            ))
            .with_note("a module presents one interface or the other, not both"));
        }
    }
    if !flags.is_empty() {
        // Both flags name something the build is FOR; they differ only in the
        // interface it presents.
        let mut keep = flags.export.clone();
        keep.extend(flags.bare.iter().cloned());
        return Ok(Exports { keep, wrap: flags.export.clone() });
    }

    let mut found = roots(modules, candidates);
    match found.len() {
        // Nothing here is a root -- every candidate is used by something that
        // is not one, which an `import` can arrange. There is no target to
        // narrow to, so the whole compilation is emitted as it lowers.
        0 => Ok(Exports {
            keep: modules.iter().map(|m| m.name.clone()).collect(),
            wrap: Vec::new(),
        }),
        1 => Ok(Exports { keep: found.clone(), wrap: found }),
        _ => {
            found.sort();
            Err(Diag::error_no_span(format!(
                "several modules could be exported: {}",
                found
                    .iter()
                    .map(|n| format!("`{}`", n))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .with_note(
                "name the ones you want with `--export <a>,<b>`, or keep the salt ports with `--bare-export <a>`",
            ))
        }
    }
}

/// Builds the wrapper for one target, and says what its logic is now called.
///
/// The wrapper computes nothing. It declares the FIFO ports, one salt net per
/// pipe, an adapter to translate between them, and one instance of the core.
/// A `wire` needs none of that and is connected straight through.
pub fn wrap(module: &Module, sig: &BlockSig, adapts: &mut Vec<AdaptUse>) -> (Module, String) {
    let core = format!("{}{}", module.name, CORE_SUFFIX);
    let mut ports = Vec::new();
    let mut nets = Vec::new();
    let mut instances = Vec::new();

    for implicit in ["clk", "rst_n"] {
        ports.push(Port { name: implicit.to_string(), dir: PortDir::In, ty: Ty::BOOL });
    }
    let mut core_conns = vec![
        ("clk".to_string(), "clk".to_string()),
        ("rst_n".to_string(), "rst_n".to_string()),
    ];

    for arg in &sig.args {
        let wire = match arg {
            ArgSig::Wire(w) => w,
            ArgSig::Pipe(p) => {
                let kind = Adapt::at(p.is_input, false);
                let use_ = AdaptUse { kind, ty: p.ty.clone() };
                if !adapts.contains(&use_) {
                    adapts.push(use_);
                }

                // The FIFO face, outward. The flag is what this module
                // answers, the `go` is what it is told, and the data goes the
                // way the items go.
                let [flag, go, data] = kind.face();
                let data_dir = if p.is_input { PortDir::In } else { PortDir::Out };
                ports.push(Port { name: format!("{}_{}", p.name, flag), dir: PortDir::Out, ty: Ty::BOOL });
                ports.push(Port { name: format!("{}_{}", p.name, go), dir: PortDir::In, ty: Ty::BOOL });
                ports.push(Port { name: format!("{}_{}", p.name, data), dir: data_dir, ty: p.ty.clone() });

                // The salt legs stay inside, which is the whole point.
                let pair = Ty::Array(Box::new(p.ty.clone()), 2);
                nets.push(Net { name: format!("{}_wsalt", p.name), ty: SALT });
                nets.push(Net { name: format!("{}_rsalt", p.name), ty: SALT });
                nets.push(Net { name: format!("{}_data", p.name), ty: pair });

                let salt = kind.salt_pipe();
                let mut conns = vec![
                    ("clk".to_string(), "clk".to_string()),
                    ("rst_n".to_string(), "rst_n".to_string()),
                    (format!("{}_wsalt", salt), format!("{}_wsalt", p.name)),
                    (format!("{}_rsalt", salt), format!("{}_rsalt", p.name)),
                    (format!("{}_data", salt), format!("{}_data", p.name)),
                ];
                for suffix in [flag, go, data] {
                    conns.push((suffix.to_string(), format!("{}_{}", p.name, suffix)));
                }
                instances.push(Instance {
                    module: crate::ir_adapt::module_name(kind, &p.ty),
                    name: format!("u_{}_adapt", p.name),
                    conns,
                    produces: Vec::new(),
                });

                for leg in ["wsalt", "rsalt", "data"] {
                    core_conns
                        .push((format!("{}_{}", p.name, leg), format!("{}_{}", p.name, leg)));
                }
                continue;
            }
        };
        ports.push(Port {
            name: wire.name.clone(),
            dir: if wire.is_input { PortDir::In } else { PortDir::Out },
            ty: wire.ty.clone(),
        });
        core_conns.push((wire.name.clone(), wire.name.clone()));
    }

    instances.push(Instance {
        module: core.clone(),
        name: format!("u_{}", core),
        conns: core_conns,
        produces: Vec::new(),
    });

    let wrapper = Module {
        // A wrapper calls nothing and is used by nothing: it is the top of
        // whatever it wraps, which is what being a target means.
        calls: Vec::new(),
        name: module.name.clone(),
        ports,
        values: Vec::new(),
        drivers: Vec::new(),
        regs: Vec::new(),
        mems: Vec::new(),
        asserts: Vec::new(),
        // Carried, not dropped: the `// Built with:` header is the only place
        // a folded constant still appears, and a wrapper whose dimensions come
        // from a number that is written down nowhere is hard to read.
        params: module.params.clone(),
        nets,
        instances,
    };
    (wrapper, core)
}

/// Renames each target's logic, repoints everything that used it, and appends
/// the wrappers.
///
/// Wrappers go last so the file still reads top to bottom: an adapter and a
/// core are both named by the wrapper that follows them.
pub fn apply(
    mut modules: Vec<Module>,
    targets: &[String],
    sigs: &BTreeMap<String, BlockSig>,
    adapts: &mut Vec<AdaptUse>,
) -> Vec<Module> {
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    let mut wrappers = Vec::new();

    for target in targets {
        let Some(module) = modules.iter().find(|m| m.name == *target) else {
            continue;
        };
        let Some(sig) = sigs.get(target) else {
            // A `fun` has no pipe interface, so there is nothing to translate
            // and nothing to wrap: it is already a plain combinational module
            // with no salt anywhere in it.
            continue;
        };
        if !sig.args.iter().any(|a| matches!(a, ArgSig::Pipe(_))) {
            continue;
        }
        let (wrapper, core) = wrap(module, sig, adapts);
        renamed.insert(target.clone(), core);
        wrappers.push(wrapper);
    }

    if !renamed.is_empty() {
        for module in &mut modules {
            if let Some(core) = renamed.get(&module.name) {
                module.name = core.clone();
            }
            for inst in &mut module.instances {
                if let Some(core) = renamed.get(&inst.module) {
                    inst.module = core.clone();
                }
            }
        }
    }
    modules.extend(wrappers);
    modules
}
