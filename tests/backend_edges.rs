use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::ir::{Instance, Module, Port, PortDir};
use ddl::ty::Ty;
use ddl::verilog::{EmitOptions, emit_modules};

#[test]
fn scalar_operations_and_collision_fixture_compile() {
    let map = SourceMap::new("edges.ddl", include_str!("probes/backend_edges.ddl"));
    let v = compile_to_verilog(&map, &EmitOptions::default())
        .unwrap_or_else(|d| panic!("{}", map.render_all(&d)));
    assert!(!v.contains("x[0]"), "{v}");
    assert!(v.contains("module cell_ ("));
    assert!(v.contains("module cell__1 ("));
    assert!(v.contains("cell_ u_cell ("));
    assert!(v.contains("cell__1 u_cell_ ("));
}

fn empty(name: &str) -> Module {
    Module {
        name: name.into(),
        ports: vec![],
        values: vec![],
        drivers: vec![],
        regs: vec![],
        mems: vec![],
        asserts: vec![],
        params: vec![],
        nets: vec![],
        instances: vec![],
    }
}
fn input(name: &str) -> Port {
    Port {
        name: name.into(),
        dir: PortDir::In,
        ty: Ty::UInt(8),
    }
}

#[test]
fn named_connections_follow_the_callee_interface_and_caller_namespace() {
    let mut leaf = empty("leaf");
    leaf.ports = vec![input("cell"), input("cell_"), input("cell__1")];
    let mut graph = empty("parent");
    graph.ports = leaf.ports.clone();
    graph.instances.push(Instance {
        module: "leaf".into(),
        name: "cell".into(),
        conns: vec![
            ("cell".into(), "cell_".into()),
            ("cell_".into(), "cell__1".into()),
            ("cell__1".into(), "cell".into()),
        ],
        produces: vec![],
    });
    let v = emit_modules(&[leaf, graph], &EmitOptions::default()).unwrap();
    assert!(v.contains(".cell_"));
    assert!(v.contains("(cell__1)"));
    assert!(v.contains(".cell__1"));
    assert!(v.contains("(cell__1_1)"));
    assert!(v.contains("leaf cell__2 ("), "{v}");
}

#[test]
fn duplicate_expanded_ports_are_diagnosed_instead_of_merged() {
    let map = SourceMap::new(
        "duplicate.ddl",
        "process p (ans: buffer out u8, ans_data: port in u8)\n  loop\n    let x = @rcv(ans_data)\n    @send(ans, x)\n",
    );
    let error = compile_to_verilog(&map, &EmitOptions::default()).unwrap_err();
    assert!(map.render_all(&error).contains("duplicate port `ans_data`"));
}

#[test]
fn ambiguous_external_and_module_declarations_are_diagnosed() {
    for (source, expected) in [
        (
            "extern ext (src: buffer in u8, src: buffer out u8)\ngraph g (a: buffer in u8, b: buffer out u8)\n  ext(a, b)\n",
            "same pipe twice",
        ),
        (
            "extern p (a: buffer in u8)\nprocess p (a: buffer in u8)\n  loop\n    let took = @drop(a)\n",
            "module `p` is declared more than once",
        ),
    ] {
        let map = SourceMap::new("duplicate.ddl", source);
        let errors = compile_to_verilog(&map, &EmitOptions::default()).unwrap_err();
        assert!(map.render_all(&errors).contains(expected));
    }
}
