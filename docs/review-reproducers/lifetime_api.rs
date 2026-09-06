use ddl::diag::SourceMap;
use ddl::driver::parse_source;
use ddl::lex::{AlphanumSpan, TopLevelDecl};
use ddl::parse::anumspan_to_str;

// Compile-only checks: no dangling memory is read by running this file.
pub fn ast_outlives_source() -> Vec<TopLevelDecl> {
    let map = SourceMap::new("example.ddl", "fun f (x: u8, o: out u8)\n  o = x\n");
    parse_source(&map).unwrap().decls
}

pub fn unconstrained_lifetime() -> &'static str {
    let text = String::from("name");
    let span = AlphanumSpan { byte_ptr: text.as_ptr(), len: text.len() as u32 };
    anumspan_to_str(&span)
}
