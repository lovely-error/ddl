// `import` end to end, against real files.
//
// The unit tests in src/source.rs cover the line rewriting; what they cannot
// cover is the part that only exists on a filesystem -- resolution order,
// `-I`, a diamond, a cycle -- and that is what breaks when someone touches the
// loader.

use std::path::{Path, PathBuf};

use ddl::diag::SourceMap;
use ddl::driver::compile_to_verilog;
use ddl::source::load_program;
use ddl::verilog::EmitOptions;

/// A scratch directory under `target/`, so a failing test leaves its inputs
/// behind to look at and `cargo clean` takes them away.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-imports").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        Scratch { dir }
    }

    fn write(&self, name: &str, body: &str) -> String {
        let path = self.dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("scratch subdirectory");
        }
        std::fs::write(&path, body).expect("scratch file");
        path.display().to_string()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

fn build(roots: &[String], search: Vec<PathBuf>) -> Result<String, String> {
    let (map, load_diags) = load_program(roots, search)?;
    if !load_diags.is_empty() {
        return Err(map.render_all(&load_diags));
    }
    compile_to_verilog(&map, &EmitOptions::default()).map_err(|d| map.render_all(&d))
}

const ADDER: &str = "fun adder (a: u8, b: u8, sum: out u8)\n  sum = a + b\n";

#[test]
fn an_imported_declaration_is_visible_without_being_named() {
    // There are no namespaces: an import says "this file is part of the
    // program", and everything in it is in scope everywhere.
    let s = Scratch::new("basic");
    s.write("types.ddl", "enum op_e: u2\n  OP_ADD\n  OP_SUB\n");
    let main = s.write(
        "main.ddl",
        "import \"types.ddl\"\n\nfun pick (o: op_e, r: out u1)\n  r = o == OP_SUB\n",
    );

    let v = build(&[main], Vec::new()).expect("should compile");
    assert!(v.contains("module pick ("), "{}", v);
    assert!(v.contains("2'd1"), "{}", v);
}

#[test]
fn an_import_resolves_against_the_importing_file_not_the_working_directory() {
    // `lib/helper.ddl` is named from `lib/mid.ddl` as `helper.ddl`, which only
    // works if resolution starts at the importer.
    let s = Scratch::new("relative");
    s.write("lib/helper.ddl", ADDER);
    s.write("lib/mid.ddl", "import \"helper.ddl\"\n");
    let main = s.write(
        "main.ddl",
        "import \"lib/mid.ddl\"\n\nfun use_it (a: u8, o: out u8)\n  o = adder(a, 8'd1)\n",
    );

    let v = build(&[main], Vec::new()).expect("should compile");
    assert!(v.contains("module use_it ("), "{}", v);
}

#[test]
fn a_search_directory_is_the_fallback() {
    // How the k2g examples reach k2g_pkg.ddl, which is generated into a
    // different repository.
    let s = Scratch::new("include-path");
    s.write("vendor/lib.ddl", ADDER);
    let main = s.write(
        "main.ddl",
        "import \"lib.ddl\"\n\nfun use_it (a: u8, o: out u8)\n  o = adder(a, 8'd1)\n",
    );

    let err = build(std::slice::from_ref(&main), Vec::new()).expect_err("not findable yet");
    assert!(err.contains("cannot find `lib.ddl`"), "{}", err);
    assert!(err.contains("main.ddl:1:8"), "{}", err);

    build(&[main], vec![s.path("vendor")]).expect("findable with -I");
}

#[test]
fn a_file_reached_twice_is_included_once() {
    // A diamond: both halves import the shared types. Including it twice would
    // be a duplicate-declaration error, which is exactly the failure mode the
    // `cat` build had.
    let s = Scratch::new("diamond");
    s.write("types.ddl", "enum op_e: u2\n  OP_ADD\n  OP_SUB\n");
    s.write(
        "left.ddl",
        "import \"types.ddl\"\n\nfun is_add (o: op_e, r: out u1)\n  r = o == OP_ADD\n",
    );
    s.write(
        "right.ddl",
        "import \"types.ddl\"\n\nfun is_sub (o: op_e, r: out u1)\n  r = o == OP_SUB\n",
    );
    let main = s.write("main.ddl", "import \"left.ddl\"\nimport \"right.ddl\"\n");

    let v = build(&[main], Vec::new()).expect("should compile");
    assert_eq!(v.matches("endmodule").count(), 2, "{}", v);
}

#[test]
fn naming_the_same_file_twice_on_the_command_line_is_harmless() {
    let s = Scratch::new("dup-roots");
    let a = s.write("a.ddl", ADDER);
    let v = build(&[a.clone(), a], Vec::new()).expect("should compile");
    assert_eq!(v.matches("endmodule").count(), 1, "{}", v);
}

#[test]
fn a_cycle_terminates() {
    let s = Scratch::new("cycle");
    s.write("a.ddl", &format!("import \"b.ddl\"\n{}", ADDER));
    let b = s.write("b.ddl", "import \"a.ddl\"\n");

    let v = build(&[b], Vec::new()).expect("should compile");
    assert_eq!(v.matches("endmodule").count(), 1, "{}", v);
}

#[test]
fn an_import_comes_before_the_file_that_asked_for_it() {
    // A `fun` is inlined at its call site, so a callee that landed after its
    // caller used to be a "not declared" error. Order is part of the contract.
    let s = Scratch::new("order");
    s.write("dep.ddl", ADDER);
    let main = s.write(
        "main.ddl",
        "fun use_it (a: u8, o: out u8)\n  o = adder(a, 8'd1)\n\nimport \"dep.ddl\"\n",
    );

    let (map, diags) = load_program(&[main], Vec::new()).expect("readable");
    assert!(diags.is_empty(), "{}", map.render_all(&diags));
    let text = map.text();
    assert!(
        text.find("fun adder").unwrap() < text.find("fun use_it").unwrap(),
        "{}",
        text
    );
}

#[test]
fn a_missing_import_points_at_the_line_that_asked_for_it() {
    let s = Scratch::new("missing");
    let main = s.write("main.ddl", "-- a comment\nimport \"nope.ddl\"\n");

    let err = build(&[main], Vec::new()).expect_err("should fail");
    assert!(err.contains("cannot find `nope.ddl`"), "{}", err);
    assert!(err.contains("main.ddl:2:8"), "{}", err);
    assert!(err.contains("`-I` directory"), "{}", err);
}

#[test]
fn the_snippet_quotes_the_import_not_the_comment_standing_in_for_it() {
    // The parser reads a blanked line; the reader must not.
    let s = Scratch::new("snippet");
    let main = s.write("main.ddl", "import \"nope.ddl\"\n");

    let err = build(&[main], Vec::new()).expect_err("should fail");
    assert!(err.contains("1 | import \"nope.ddl\""), "{}", err);
    // And the caret sits under the path, not under the keyword.
    assert!(err.contains("|        ^^^^^^^^^^"), "{}", err);
}

#[test]
fn a_root_that_does_not_exist_says_so_plainly() {
    let err = build(&["no/such/file.ddl".to_string()], Vec::new()).expect_err("should fail");
    assert!(err.contains("cannot read"), "{}", err);
}

#[test]
fn an_error_in_an_imported_file_names_that_file_and_its_own_line_number() {
    // The whole point of concatenating into one buffer with a file table: the
    // `cat` build reported this at line 43 of a temporary nobody wrote.
    let s = Scratch::new("blame");
    s.write("dep.ddl", "fun broken (a: u8, b: u32, o: out u8)\n  o = a + b\n");
    let main = s.write("main.ddl", "import \"dep.ddl\"\n");

    let err = build(&[main], Vec::new()).expect_err("should fail");
    assert!(err.contains("width mismatch"), "{}", err);
    assert!(err.contains("dep.ddl:"), "{}", err);
    assert!(!err.contains("main.ddl:"), "{}", err);
}

#[test]
fn offsets_after_an_import_line_are_the_offsets_in_the_file() {
    // Blanking the import line to a comment of the same length is what keeps
    // this true, and a column that is off by the length of a path is the kind
    // of wrongness that gets noticed late.
    let s = Scratch::new("offsets");
    s.write("a/very/long/path/to/types.ddl", "enum op_e: u2\n  OP_ADD\n");
    let main = s.write(
        "main.ddl",
        "import \"a/very/long/path/to/types.ddl\"\nfun f (a: u8, o: out u8)\n  o = nope\n",
    );

    // Line 3 column 7 is where `nope` starts, and it stays there however long
    // the import line above it happens to be.
    let err = build(&[main], Vec::new()).expect_err("should fail");
    assert!(err.contains("main.ddl:3:7"), "{}", err);
}

#[test]
fn several_inputs_compile_as_one_program() {
    // What `ddl build a.ddl b.ddl` has to mean, and what verify.sh used `cat`
    // for before there was an import.
    let s = Scratch::new("several");
    let types = s.write("types.ddl", "enum op_e: u2\n  OP_ADD\n  OP_SUB\n");
    let user = s.write("user.ddl", "fun pick (o: op_e, r: out u1)\n  r = o == OP_SUB\n");

    let v = build(&[types, user], Vec::new()).expect("should compile");
    assert!(v.contains("module pick ("), "{}", v);
}

#[test]
fn one_banner_covers_the_whole_program() {
    let s = Scratch::new("banner");
    let a = s.write("a.ddl", ADDER);
    let b = s.write("b.ddl", "fun other (a: u8, o: out u8)\n  o = a\n");

    let v = build(&[a, b], Vec::new()).expect("should compile");
    assert_eq!(v.matches("GENERATED FILE").count(), 1, "{}", v);
    assert_eq!(v.matches("endmodule").count(), 2, "{}", v);
}

#[test]
fn a_single_file_program_is_unchanged_by_all_of_this() {
    let s = Scratch::new("plain");
    let a = s.write("a.ddl", ADDER);

    let through_loader = build(std::slice::from_ref(&a), Vec::new()).expect("should compile");
    let direct = {
        let map = SourceMap::new(a, ADDER);
        compile_to_verilog(&map, &EmitOptions::default()).expect("should compile")
    };
    assert_eq!(through_loader, direct);
}
