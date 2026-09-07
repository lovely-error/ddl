use ddl::diag::SourceMap;
use ddl::lex::{AlphanumSpan, TopLevelDecl};
use ddl::parse::anumspan_to_str;

#[test]
fn extracted_ast_owns_identifiers_and_string_literals() {
    let decls = {
        let map = SourceMap::new("owned.ddl", "process owned (o: port out u8)\n  loop\n    @assert(1'b1, \"owned message\")\n    @try_send(o, 8'd1)\n");
        ddl::driver::parse_source(&map).unwrap().decls
    };
    // Both identifier formatting and string literal formatting used to read
    // freed SourceMap storage. The AST now owns both kinds of text.
    let dump = format!("{decls:?}");
    assert!(dump.contains("owned message"));
    let TopLevelDecl::ProcessStmt(p) = &decls[0] else { panic!("process") };
    assert_eq!(anumspan_to_str(&p.name), "owned");
    let cloned = p.name.clone();
    drop(decls);
    assert_eq!(anumspan_to_str(&cloned), "owned");
}

#[test]
fn public_location_metadata_cannot_make_text_access_dereference_a_pointer() {
    let mut span = AlphanumSpan::new(String::from("safe"));
    span.byte_ptr = std::ptr::null();
    span.len = u32::MAX;
    assert_eq!(anumspan_to_str(&span), "safe");
    assert_eq!(format!("{span:?}"), "`safe`");
}
