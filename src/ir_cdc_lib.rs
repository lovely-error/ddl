// The clock-domain crossing, as modules the compiler EMITS rather than lowers.
//
// Everything else the compiler writes -- adapters in `ir_adapt.rs`, combinators
// in `ir_comb.rs` -- is built as IR and rendered by the backend. A crossing is
// not, and the difference is deliberate.
//
// A crossing has registers on two clocks. Lowering one into IR would mean
// teaching `Reg` which clock it belongs to and teaching `emit_clocked_block` to
// group by it -- a change to the shape of every module in the compiler, in
// service of one module that never varies. Worse, it would mean the crossing
// shipped to a user is logic the compiler assembled per width, verified by
// argument.
//
// So the crossing is a fixed file, carried here by `include_str!` and written
// out verbatim. The bytes emitted are the bytes reviewed, and they are the same
// bytes `cdc-demo/` measured on a Tang Nano 9K at zero errors and full rate.
// `include_str!` also means a moved or renamed file fails `cargo build` rather
// than silently shipping something stale.
//
// What IS generated is a thin shell per shape, twenty lines of obvious Verilog,
// holding the three things a wrapper cannot: the parameter override, one AND
// gate of handshake glue, and the reset handshake instance. The part that is
// hard to get right is the part nobody rewrites; the part that is generated is
// the part you can check by reading it.

/// The measured async FIFO. `lib/ddl_cdc_fifo.v`, verbatim.
pub const CDC_FIFO_V: &str = include_str!("../lib/ddl_cdc_fifo.v");

/// The reset handshake. `lib/ddl_rst_cross.v`, verbatim.
pub const RST_CROSS_V: &str = include_str!("../lib/ddl_rst_cross.v");

/// Which way a crossing faces, and who drives the handshake on each side.
///
/// The same four-way split `Adapt` makes, and for the same reason: master
/// versus slave is decided by who the shell faces, and the difference is one
/// AND gate. An export target presents its face outward and lets whoever
/// instantiates it drive; an `extern` is driven BY the graph, so the shell is
/// the master there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cdc {
    /// An export target's `buffer in`: the outside writes, DDL consumes.
    In,
    /// An export target's `buffer out`: DDL produces, the outside reads.
    Out,
    /// An `extern`'s `buffer in`: DDL sends, the extern consumes.
    ToExtern,
    /// An `extern`'s `buffer out`: the extern produces, DDL consumes.
    FromExtern,
}

impl Cdc {
    /// Which crossing belongs at one pipe, mirroring `Adapt::at`.
    pub fn at(is_input: bool, foreign: bool) -> Cdc {
        match (is_input, foreign) {
            (true, false) => Cdc::In,
            (false, false) => Cdc::Out,
            (true, true) => Cdc::ToExtern,
            (false, true) => Cdc::FromExtern,
        }
    }

    fn what(&self) -> &'static str {
        match self {
            Cdc::In => "in",
            Cdc::Out => "out",
            Cdc::ToExtern => "to_ext",
            Cdc::FromExtern => "from_ext",
        }
    }
}

/// One crossing a boundary asked for: which way, how wide, how deep.
///
/// Keyed by shape and not by use, exactly as `AdaptUse` is -- and with the same
/// hazard recorded there: the dedup key has to be what the NAME is, or two
/// boundaries produce one module name from two different shapes and the design
/// is rejected for defining it twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdcUse {
    pub kind: Cdc,
    pub width: u32,
    pub depth: u32,
}

/// The shell's module name. Width and depth are both in it because both change
/// the module.
pub fn module_name(kind: Cdc, width: u32, depth: u32) -> String {
    format!("ddl_cdc_{}_{}x{}", kind.what(), width, depth)
}

/// `log2(depth)`, which is what `ddl_cdc_fifo` takes. Depth is validated as a
/// power of two at least 4 before it reaches here.
fn addr_width(depth: u32) -> u32 {
    depth.trailing_zeros()
}

/// Builds one shell.
///
/// The two sides are named `f_*` for the foreign face and `c_*` for the side
/// that faces the adapter DDL already emits, so a reader can tell at a glance
/// which clock a signal belongs to.
pub fn shell(kind: Cdc, width: u32, depth: u32) -> String {
    let name = module_name(kind, width, depth);
    let aw = addr_width(depth);
    let hi = width - 1;

    // Which side of the FIFO runs on which clock, and which face is master.
    //
    //   In / FromExtern   data flows INTO the core: the FIFO is written on the
    //                     foreign clock and read on `clk`.
    //   Out / ToExtern    data flows OUT of the core: written on `clk`, read on
    //                     the foreign clock.
    let into_core = matches!(kind, Cdc::In | Cdc::FromExtern);

    let mut s = String::new();
    s.push_str(&format!(
        "// GENERATED. One clock-domain crossing, {} bits deep {}.\n\
         //\n\
         // A parameter override, one AND gate of glue, and the reset handshake.\n\
         // The crossing itself is `ddl_cdc_fifo`, which is a fixed reviewed file --\n\
         // see its header, and docs/clock-domains.md.\n\
         module {} (\n\
         \x20   input  clk,\n\
         \x20   input  rst_n,\n\
         \x20   input  f_clk,\n",
        width,
        depth,
        name
    ));

    match kind {
        // The outside writes in; the shell drives the adapter's slave write face.
        Cdc::In => s.push_str(&format!(
            "\x20   // foreign side: a slave write face, driven from f_clk\n\
             \x20   output        f_can_receive,\n\
             \x20   input         f_receive_en,\n\
             \x20   input  [{hi}:0] f_data_write_in,\n\
             \x20   // core side: a master write face into the adapter\n\
             \x20   input         c_can_receive,\n\
             \x20   output        c_receive_en,\n\
             \x20   output [{hi}:0] c_data_write_in\n"
        )),
        // DDL produces; the shell pulls the adapter and offers outward.
        Cdc::Out => s.push_str(&format!(
            "\x20   // core side: a master read face pulling the adapter\n\
             \x20   input         c_has_data,\n\
             \x20   output        c_drop_item,\n\
             \x20   input  [{hi}:0] c_data_read_out,\n\
             \x20   // foreign side: a slave read face, read from f_clk\n\
             \x20   output        f_has_data,\n\
             \x20   input         f_drop_item,\n\
             \x20   output [{hi}:0] f_data_read_out\n"
        )),
        // DDL sends to an extern: the adapter is the master, so this is a slave
        // write face on the core side and a master write face into the extern.
        Cdc::ToExtern => s.push_str(&format!(
            "\x20   // core side: a slave write face, driven by the adapter\n\
             \x20   output        c_can_receive,\n\
             \x20   input         c_receive_en,\n\
             \x20   input  [{hi}:0] c_data_write_in,\n\
             \x20   // foreign side: a master write face into the extern\n\
             \x20   input         f_can_receive,\n\
             \x20   output        f_receive_en,\n\
             \x20   output [{hi}:0] f_data_write_in\n"
        )),
        // An extern produces: a master read face reading it, a slave read face
        // for the adapter to pull.
        Cdc::FromExtern => s.push_str(&format!(
            "\x20   // foreign side: a master read face reading the extern\n\
             \x20   input         f_has_data,\n\
             \x20   output        f_drop_item,\n\
             \x20   input  [{hi}:0] f_data_read_out,\n\
             \x20   // core side: a slave read face for the adapter to pull\n\
             \x20   output        c_has_data,\n\
             \x20   input         c_drop_item,\n\
             \x20   output [{hi}:0] c_data_read_out\n"
        )),
    }

    s.push_str(");\n\n");
    s.push_str(
        "  // Both halves leave zero together, at any clock ratio and however\n\
         \x20 // short the pulse on rst_n. See lib/ddl_rst_cross.v.\n\
         \x20 wire w_rst_n, r_rst_n;\n\
         \x20 ddl_rst_cross u_rst (\n\
         \x20     .clk(clk), .rst_n(rst_n), .f_clk(f_clk),\n\
         \x20     .w_rst_n(w_rst_n), .r_rst_n(r_rst_n)\n\
         \x20 );\n\n",
    );
    s.push_str(&format!(
        "  wire        wfull, rempty;\n\
         \x20 wire [{hi}:0] rdata;\n\
         \x20 wire        wpush, rpop;\n\
         \x20 wire [{hi}:0] wdata;\n\n"
    ));

    // The FIFO's write side sits on whichever clock the data comes FROM.
    let (wclk, wrst, rclk, rrst) = if into_core {
        ("f_clk", "r_rst_n", "clk", "w_rst_n")
    } else {
        ("clk", "w_rst_n", "f_clk", "r_rst_n")
    };
    s.push_str(&format!(
        "  ddl_cdc_fifo #(.WIDTH({width}), .AW({aw})) u_fifo (\n\
         \x20     .wclk({wclk}), .wrst_n({wrst}), .wpush(wpush), .wdata(wdata), .wfull(wfull),\n\
         \x20     .rclk({rclk}), .rrst_n({rrst}), .rpop(rpop),   .rdata(rdata), .rempty(rempty)\n\
         \x20 );\n\n"
    ));

    // ...and the glue, which is the only arithmetic in the file.
    s.push_str(match kind {
        Cdc::In =>
            "  assign wpush           = f_receive_en;\n\
             \x20 assign wdata           = f_data_write_in;\n\
             \x20 assign f_can_receive   = !wfull;\n\n\
             \x20 assign c_receive_en    = !rempty && c_can_receive;   // the only glue\n\
             \x20 assign c_data_write_in = rdata;\n\
             \x20 assign rpop            = c_receive_en;\n",
        Cdc::Out =>
            "  assign c_drop_item     = c_has_data && !wfull;        // the only glue\n\
             \x20 assign wpush           = c_drop_item;\n\
             \x20 assign wdata           = c_data_read_out;\n\n\
             \x20 assign f_has_data      = !rempty;\n\
             \x20 assign f_data_read_out = rdata;\n\
             \x20 assign rpop            = f_drop_item;\n",
        Cdc::ToExtern =>
            "  assign wpush           = c_receive_en;\n\
             \x20 assign wdata           = c_data_write_in;\n\
             \x20 assign c_can_receive   = !wfull;\n\n\
             \x20 assign f_receive_en    = !rempty && f_can_receive;   // the only glue\n\
             \x20 assign f_data_write_in = rdata;\n\
             \x20 assign rpop            = f_receive_en;\n",
        Cdc::FromExtern =>
            "  assign f_drop_item     = f_has_data && !wfull;        // the only glue\n\
             \x20 assign wpush           = f_drop_item;\n\
             \x20 assign wdata           = f_data_read_out;\n\n\
             \x20 assign c_has_data      = !rempty;\n\
             \x20 assign c_data_read_out = rdata;\n\
             \x20 assign rpop            = c_drop_item;\n",
    });
    s.push_str("\nendmodule\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shape_names_the_module_and_the_name_is_the_dedup_key() {
        assert_eq!(module_name(Cdc::In, 16, 8), "ddl_cdc_in_16x8");
        assert_eq!(module_name(Cdc::Out, 32, 4), "ddl_cdc_out_32x4");
        // Same width, different depth, different module -- the hazard
        // ir_adapt.rs records is that the key must be what the name is.
        assert_ne!(module_name(Cdc::In, 16, 8), module_name(Cdc::In, 16, 16));
    }

    #[test]
    fn depth_becomes_the_fifos_address_width() {
        assert_eq!(addr_width(4), 2);
        assert_eq!(addr_width(8), 3);
        assert_eq!(addr_width(16), 4);
        assert!(shell(Cdc::In, 16, 8).contains(".AW(3)"));
        assert!(shell(Cdc::In, 16, 16).contains(".AW(4)"));
    }

    #[test]
    fn the_fifo_is_written_on_the_clock_the_data_comes_from() {
        // Into the core: written on the foreign clock, read on clk.
        let into = shell(Cdc::In, 8, 8);
        assert!(into.contains(".wclk(f_clk)"), "{}", into);
        assert!(into.contains(".rclk(clk)"), "{}", into);
        // Out of the core: the other way round.
        let out = shell(Cdc::Out, 8, 8);
        assert!(out.contains(".wclk(clk)"), "{}", out);
        assert!(out.contains(".rclk(f_clk)"), "{}", out);
    }

    #[test]
    fn every_shell_instantiates_the_reviewed_files_and_nothing_else() {
        for kind in [Cdc::In, Cdc::Out, Cdc::ToExtern, Cdc::FromExtern] {
            let s = shell(kind, 16, 8);
            assert!(s.contains("ddl_cdc_fifo #("), "{:?}", kind);
            assert!(s.contains("ddl_rst_cross u_rst"), "{:?}", kind);
            assert!(s.contains("endmodule"), "{:?}", kind);
        }
    }

    #[test]
    fn the_carried_sources_are_the_reviewed_files() {
        assert!(CDC_FIFO_V.contains("module ddl_cdc_fifo"));
        assert!(RST_CROSS_V.contains("module ddl_rst_cross"));
        // The attributes are the whole reason the file is carried verbatim
        // rather than regenerated.
        assert!(CDC_FIFO_V.contains("ASYNC_REG"));
        assert!(CDC_FIFO_V.contains("syn_preserve"));
    }
}
