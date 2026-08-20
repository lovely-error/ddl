// A design as a picture.
//
// A `graph` is the one declaration whose meaning IS its shape, and the two
// existing ways to read one are both indirect: the source lists instances and
// leaves the connections to be matched up by pipe name, and the Verilog lists
// them again with three wires per pipe in between. Neither shows you the
// shape. On six instances that is an annoyance; on a real design it is the
// difference between seeing a feedback path and not.
//
// Graphviz because it is text: it goes in a commit, diffs when the design
// changes, and needs no tool to be useful -- the source of a dot file is
// readable on its own, which is more than can be said for a screenshot.

use crate::ir::{Module, PortDir};

/// Renders every graph in the program as one directed graph.
///
/// Modules with no instances are skipped rather than drawn as lone boxes: a
/// `fun` or a `process` has no structure to show, and one node per module
/// would bury the graphs that do.
pub fn render(modules: &[Module]) -> String {
    let mut out = String::new();
    out.push_str("digraph ddl {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [fontname=\"monospace\"];\n");
    out.push_str("  edge [fontname=\"monospace\", fontsize=10];\n");

    let mut drawn = 0;
    for module in modules {
        if module.instances.is_empty() {
            continue;
        }
        render_graph(&mut out, module);
        drawn += 1;
    }

    if drawn == 0 {
        // Not an error, and worth saying: `--emit=dot` on a file of pure `fun`
        // declarations produces an empty picture, and an empty picture looks
        // exactly like a broken tool.
        out.push_str("  empty [shape=none, label=\"no `graph` declarations\"];\n");
    }
    out.push_str("}\n");
    out
}

fn render_graph(out: &mut String, module: &Module) {
    out.push_str(&format!("\n  subgraph cluster_{} {{\n", sanitize(&module.name)));
    out.push_str(&format!("    label=\"graph {}\";\n", module.name));
    out.push_str("    style=rounded;\n");
    out.push_str("    color=gray50;\n");

    // The graph's own pipes, as the boundary. Drawn differently from an
    // instance because they are where the design meets whatever is outside it.
    for (name, dir) in boundary_pipes(module) {
        let shape = if dir == PortDir::In { "invhouse" } else { "house" };
        out.push_str(&format!(
            "    \"{}__{}\" [shape={}, label=\"{}\"];\n",
            module.name, name, shape, name
        ));
    }

    for inst in &module.instances {
        out.push_str(&format!(
            "    \"{}__{}\" [shape=box, label=\"{}\\n{}\"];\n",
            module.name, inst.name, inst.name, inst.module
        ));
    }

    // One edge per pipe, not one per wire. Three edges labelled valid, ready
    // and data would say nothing the reader does not already know about how a
    // pipe is built, and would triple the picture.
    for edge in edges(module) {
        out.push_str(&format!(
            "    \"{}__{}\" -> \"{}__{}\" [label=\"{}\"];\n",
            module.name, edge.from, module.name, edge.to, edge.pipe
        ));
    }
    out.push_str("  }\n");
}

/// The graph's parameters, by pipe name and direction.
///
/// Derived from the ports rather than kept separately: a pipe is three ports
/// sharing a prefix, and `_valid` is the one whose direction says which way
/// the pipe goes -- `_ready` points the other way and `_data` carries a type.
fn boundary_pipes(module: &Module) -> Vec<(String, PortDir)> {
    module
        .ports
        .iter()
        .filter_map(|p| {
            p.name
                .strip_suffix("_valid")
                .map(|base| (base.to_string(), p.dir))
        })
        .collect()
}

struct Edge {
    from: String,
    to: String,
    pipe: String,
}

/// One edge per pipe, from whatever drives it to whatever reads it.
///
/// The direction comes from the instance's own port direction, which the
/// connection list already carries -- an instance whose `_valid` formal is an
/// output is the producer. That is the same fact `check_endpoints` used to
/// prove there is exactly one of each, so the picture cannot disagree with the
/// design that was checked.
fn edges(module: &Module) -> Vec<Edge> {
    let mut producer: Vec<(String, String)> = Vec::new();
    let mut consumer: Vec<(String, String)> = Vec::new();

    for inst in &module.instances {
        for (_, actual) in &inst.conns {
            let Some(pipe) = actual.strip_suffix("_valid") else {
                continue;
            };
            if inst.produces.iter().any(|p| p == pipe) {
                producer.push((pipe.to_string(), inst.name.clone()));
            } else {
                consumer.push((pipe.to_string(), inst.name.clone()));
            }
        }
    }

    // A pipe that is a parameter of the graph has one end outside it; the
    // boundary node stands in for that end.
    let boundary: Vec<(String, PortDir)> = boundary_pipes(module);
    for (name, dir) in &boundary {
        if *dir == PortDir::In {
            producer.push((name.clone(), name.clone()));
        } else {
            consumer.push((name.clone(), name.clone()));
        }
    }

    let mut edges = Vec::new();
    for (pipe, from) in &producer {
        for (other, to) in &consumer {
            if other == pipe {
                edges.push(Edge { from: from.clone(), to: to.clone(), pipe: pipe.clone() });
            }
        }
    }
    edges
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
