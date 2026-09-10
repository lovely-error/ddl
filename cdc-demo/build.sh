#!/usr/bin/env bash
# Build (and optionally flash) one arm of the CDC demonstration.
#
#   bash cdc-demo/build.sh a            control: both sides on clock A
#   bash cdc-demo/build.sh b            the crossing as the compiler emits it
#   bash cdc-demo/build.sh c            the same, with 2-flop synchronizers
#   bash cdc-demo/build.sh e            a proven depth-8 async FIFO instead
#   bash cdc-demo/build.sh g            the crossing the COMPILER writes and wires
#
#   bash cdc-demo/build.sh b --pair 1   the same arm at another frequency pair
#   bash cdc-demo/build.sh b --flash    build, then load over JTAG into SRAM
#
# Reduced from KAMASUTRA2G/rtl/board/build.sh, keeping the four things that
# file learned the hard way and this one would otherwise relearn:
#
#  (each is marked at its site below)
#
#   1. capture gw_sh THROUGH A PIPE, not a redirect -- redirected, every
#      failing build produces an empty log and looks like a silent crash;
#   2. THE BITSTREAM IS THE VERDICT, not the exit code, which is 1 on builds
#      that produced a perfectly good bitstream;
#   3. PRINT WARNINGS ON SUCCESS, because an ODIV_SEL the tool silently
#      substitutes is reported nowhere else and changes the clock you think
#      you are running;
#   4. read timing from project_tr_content.html, not project.tr.html, which is
#      only a frameset.
#
# And one this experiment adds: CHECK THE NETLIST. Attributes are a request,
# not a guarantee, and a DUT the tool optimised away would look exactly like a
# passing arm B -- the null result we would most easily believe and most regret.

set -u

GW_SH="${GW_SH:-C:/Gowin/Gowin_V1.9.12.02_SP2_x64/IDE/bin/gw_sh.exe}"
PROGRAMMER="${PROGRAMMER:-C:/Gowin/Gowin_V1.9.12.02_SP2_x64/Programmer/bin/programmer_cli.exe}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
DDL="${DDL:-$ROOT/target/release/ddl.exe}"

ARM="${1:-}"
shift || true
PAIR=0
FLASH=0
while [ $# -gt 0 ]; do
    case "$1" in
        --pair)  PAIR="$2"; shift 2 ;;
        --flash) FLASH=1; shift ;;
        *) echo "unknown option '$1'" >&2; exit 1 ;;
    esac
done

case "$ARM" in
    a|b|c|e|f|g) ;;
    *) echo "usage: build.sh <a|b|c|e|f|g> [--pair 0|1|2] [--flash]" >&2; exit 1 ;;
esac

[ -x "$GW_SH" ] || { echo "gw_sh not found at $GW_SH (set GW_SH=<path>)" >&2; exit 1; }
[ -x "$DDL" ]   || { echo "ddl not found at $DDL -- cargo build --release (set DDL=<path>)" >&2; exit 1; }

# ---- the frequency pair -----------------------------------------------------
#
# Both clocks come off the one 27 MHz crystal, so they are coherent and their
# relative phase takes a few fixed values rather than sweeping. Arm B therefore
# either fails constantly or not at all depending on where placement puts the
# crossing relative to that alignment -- a coin flip, not a probability. Three
# pairs is three rebuilds and no new code.
#
#   CLKOUT = 27 * (FBDIV+1) / (IDIV+1),  VCO = CLKOUT * ODIV (400..1200 MHz)
#
# ODIV_SEL IS NOT FREE, and this cost a build to establish. KAMASUTRA2G's
# k2g_pll.sv records ODIV 10 and ODIV 6 as working and ODIV 8 as failing; on this
# toolchain and device the opposite holds. Asking for 10 gets
#
#   WARN (EX0205) : parameter "ODIV_SEL" value invalid, replaced by default "8"
#
# in an otherwise green build -- so the part runs at a VCO nothing in the source
# names. rPLL takes ODIV_SEL only from a fixed set, and 8 is in it while 6 and 10
# are not. Every frequency below therefore uses ODIV 8, which keeps the VCO
# (CLKOUT * ODIV) inside 400..1200 MHz in all four cases:
#
#   81   MHz * 8 = 648      108  MHz * 8 = 864
#   67.5 MHz * 8 = 540       94.5 MHz * 8 = 756
#
# WHY 108 AND NOT 135. At 135 MHz the reference FIFO's own empty path --
# rbin -> increment -> gray -> compare -> rempty, the lookahead formulation --
# closes at 130 MHz on this part at the slow corner, so arm E missed timing
# while B and C met it. A positive control that does not meet timing is not a
# control, and arms that are not all at the same clocks are not comparable.
# Every pair here therefore keeps both clocks at or under 108 MHz.
#
# build.sh treats a substituted divider as a failed build, not a warning.
case "$PAIR" in
    0) A_IDIV=0 A_FBDIV=2 A_ODIV=8 A_MUL=3 A_DIV=1 A_HZ=81000000
       B_IDIV=0 B_FBDIV=3 B_ODIV=8 B_MUL=4 B_DIV=1 B_HZ=108000000 ;;    # 81 / 108,  4:3
    1) A_IDIV=0 A_FBDIV=2 A_ODIV=8 A_MUL=3 A_DIV=1 A_HZ=81000000
       B_IDIV=1 B_FBDIV=6 B_ODIV=8 B_MUL=7 B_DIV=2 B_HZ=94500000 ;;     # 81 / 94.5, 7:6
    2) A_IDIV=1 A_FBDIV=4 A_ODIV=8 A_MUL=5 A_DIV=2 A_HZ=67500000
       B_IDIV=0 B_FBDIV=3 B_ODIV=8 B_MUL=4 B_DIV=1 B_HZ=108000000 ;;    # 67.5 / 108, 8:5
    *) echo "unknown pair '$PAIR' (0, 1 or 2)" >&2; exit 1 ;;
esac

# Arm A is one clock, so the report clock is clock A.
if [ "$ARM" = "a" ]; then REPORT_HZ=$A_HZ; else REPORT_HZ=$B_HZ; fi
UART_DIVISOR=$(( (REPORT_HZ + 57600) / 115200 ))

WORK="$HERE/out/$ARM-pair$PAIR"
rm -rf "$WORK"; mkdir -p "$WORK"

# ---- DUT-2, from source every time ------------------------------------------
#
# Arm G compiles the SAME source with --export and two named crossings instead.
# One command is the whole difference between it and arm E: the emitted file
# then carries ddl_cdc_fifo, ddl_rst_cross and the two generated shells, and the
# top wires nothing but clocks. Nothing here edits that file -- what the
# compiler wrote is what goes on the board.
if [ "$ARM" = "g" ]; then
    "$DDL" build "$HERE/gen_chk.ddl" --export gen,chk         --async-export gen.o=rx --async-export chk.src=tx         -o "$WORK/gen_chk_async.v" || {
        echo "DDL compilation failed" >&2; exit 1; }
else
    "$DDL" build "$HERE/gen_chk.ddl" --bare-export gen,chk -o "$WORK/gen_chk.v" || {
        echo "DDL compilation failed" >&2; exit 1; }
fi

# ---- the PLLs, with the dividers as LITERALS --------------------------------
#
# Not as module parameters. GowinSynthesis does not evaluate a parameter
# forwarded into rPLL -- it substitutes the default and says so only as
# `WARN (EX0205) ... replaced by default value`, leaving exit status, bitstream,
# utilization and timing all green while the part runs at a frequency nothing in
# the source names. For this experiment that would silently change the clock
# ratio being measured. See cdc_pll.v.in.
sed -e "s/@NAME@/cdc_pll_a/" -e "s/@IDIV@/$A_IDIV/" -e "s/@FBDIV@/$A_FBDIV/" -e "s/@ODIV@/$A_ODIV/"     "$HERE/cdc_pll.v.in" > "$WORK/cdc_pll_gen.v"
if [ "$ARM" != "a" ]; then
    sed -e "s/@NAME@/cdc_pll_b/" -e "s/@IDIV@/$B_IDIV/" -e "s/@FBDIV@/$B_FBDIV/" -e "s/@ODIV@/$B_ODIV/"         "$HERE/cdc_pll.v.in" | sed -n '/^module /,$p' >> "$WORK/cdc_pll_gen.v"
fi

# ---- what varies between arms ----------------------------------------------
{
    echo "// GENERATED BY build.sh -- arm $ARM, frequency pair $PAIR."
    case "$ARM" in
        a) echo '`define ARM_A'
           echo '`define ARM_CHAR 8'"'"'h41'
           echo '`define SLOT1_SYNC 0'
           echo '`define SLOT1_DERIVE 0' ;;
        b) echo '`define ARM_B'
           echo '`define ARM_CHAR 8'"'"'h42'
           echo '`define SLOT1_SYNC 0'
           echo '`define SLOT1_DERIVE 0' ;;
        c) echo '`define ARM_C'
           echo '`define ARM_CHAR 8'"'"'h43'
           echo '`define SLOT1_SYNC 1'
           echo '`define SLOT1_DERIVE 0'
           echo '`define SLOT2_SYNC' ;;
        e) echo '`define ARM_E'
           echo '`define ARM_CHAR 8'"'"'h45'
           echo '`define SLOT1_SYNC 0'
           echo '`define SLOT1_DERIVE 0' ;;
        g) echo '`define ARM_G'
           echo '`define ARM_CHAR 8'"'"'h47'
           echo '`define SLOT1_SYNC 0'
           echo '`define SLOT1_DERIVE 0' ;;
        f) echo '`define ARM_F'
           echo '`define ARM_CHAR 8'"'"'h46'
           echo '`define SLOT1_SYNC 0'
           echo '`define SLOT1_DERIVE 1' ;;
    esac
    echo "\`define PLL_A_IDIV  $A_IDIV"
    echo "\`define PLL_A_FBDIV $A_FBDIV"
    echo "\`define PLL_A_ODIV  $A_ODIV"
    echo "\`define PLL_B_IDIV  $B_IDIV"
    echo "\`define PLL_B_FBDIV $B_FBDIV"
    echo "\`define PLL_B_ODIV  $B_ODIV"
    echo "\`define UART_DIVISOR $UART_DIVISOR"
} > "$WORK/arm_config.vh"

# ---- timing, self-contained per arm ----------------------------------------
#
# A stage file REPLACES a shared one: Gowin will not resolve a clock created in
# another file and fails with "Cannot get clock with name". PLL outputs are
# declared as GENERATED clocks chained from the crystal, not as base clocks --
# a base clock has no master, so the tool stops knowing they share a source.
#
# set_clock_groups MUST BE ONE LINE; the SDC parser takes no continuation.
{
    echo "// GENERATED BY build.sh -- arm $ARM, frequency pair $PAIR."
    echo "create_clock -name clk_27m -period 37.037 -waveform {0 18.5} [get_ports {clk_27m}]"
    echo "create_generated_clock -name clk_a -source [get_ports {clk_27m}] -multiply_by $A_MUL -divide_by $A_DIV [get_nets {clk_a}]"
    if [ "$ARM" != "a" ]; then
        echo "create_generated_clock -name clk_b -source [get_ports {clk_27m}] -multiply_by $B_MUL -divide_by $B_DIV [get_nets {clk_b}]"
        echo ""
        echo "// The crossing is asynchronous, which is what a real two-domain design"
        echo "// declares -- and what stops the tool timing these paths and quietly"
        echo "// closing them. An arm B that passes because the tool closed the crossing"
        echo "// has measured nothing."
        echo "set_clock_groups -asynchronous -group [get_clocks {clk_27m}] -group [get_clocks {clk_a}] -group [get_clocks {clk_b}]"
    else
        echo "set_clock_groups -asynchronous -group [get_clocks {clk_27m}] -group [get_clocks {clk_a}]"
    fi
} > "$WORK/arm.sdc"

# ---- sources ----------------------------------------------------------------
SRC=("$WORK/cdc_pll_gen.v" "$HERE/cdc_uart.v" "$HERE/cdc_demo_top.v")
if [ "$ARM" = "e" ]; then
    SRC+=("$HERE/cdc_fifo.v")
elif [ "$ARM" = "g" ]; then
    # One file, and the compiler wrote all of it -- the two wrappers, the two
    # generated shells, ddl_cdc_fifo and ddl_rst_cross verbatim.
    SRC+=("$WORK/gen_chk_async.v")
else
    SRC+=("$HERE/pipe_cdc.v" "$WORK/gen_chk.v")
fi
for f in "${SRC[@]}"; do cp "$f" "$WORK/"; done
cp "$HERE/tangnano9k.cst" "$WORK/board.cst"

{
    echo "set_device -name GW1NR-9C GW1NR-LV9QN88PC6/I5"
    for f in "${SRC[@]}"; do echo "add_file -type verilog {$(basename "$f")}"; done
    echo "add_file -type cst {board.cst}"
    echo "add_file -type sdc {arm.sdc}"
    echo "set_option -top_module cdc_demo_top"
    echo "set_option -verilog_std v2001"
    echo "set_option -include_path {.}"
    # Retiming would move logic across the salt registers and relocate the
    # asynchronous boundary. Off explicitly rather than assumed off.
    echo "set_option -retiming 0"
    echo "set_option -gen_verilog_sim_netlist 1"
    # Leave unused pins defined rather than floating.
    echo "set_option -use_mspi_as_gpio 1"
    echo "set_option -use_sspi_as_gpio 1"
    echo "set_option -use_ready_as_gpio 1"
    echo "set_option -use_done_as_gpio 1"
    echo "set_option -use_reconfign_as_gpio 1"
    echo "run all"
} > "$WORK/build.tcl"

echo "building arm '$ARM' at pair $PAIR (clock A $A_HZ Hz, clock B $REPORT_HZ Hz) ..."

# 1. THROUGH A PIPE, NOT A REDIRECT. `> file 2>&1` captures NOTHING from gw_sh,
#    so every failing build produces an empty log and looks like the silent
#    toolchain crash you would then bisect blindly. It is not silent in a pipe.
set -o pipefail
( cd "$WORK" && "$GW_SH" build.tcl ) 2>&1 | tee "$WORK/build.log"
rc=${PIPESTATUS[0]}
set +o pipefail

# 2. THE BITSTREAM IS THE VERDICT. gw_sh exits 1 on builds that produced a
#    perfectly good bitstream, so `rc` alone reports failures that did not
#    happen. It is reported as evidence, never used to declare one.
FS="$WORK/impl/pnr/project.fs"
if [ ! -f "$FS" ] || [ "$FS" -ot "$WORK/build.tcl" ]; then
    echo "BUILD FAILED (gw_sh exit $rc)"
    echo "No fresh bitstream. gw_sh's output is not captured when redirected -- to see it:"
    echo "    cd $WORK && \"\$GW_SH\" build.tcl"
    grep -iE '^ERROR|\bERROR \(' "$WORK/build.log" 2>/dev/null | head -12
    exit 1
fi

# ---- a substituted PLL divider is a failed experiment -----------------------
#
# EX0205 means the tool rejected a divider and used its own. The build is green
# in every other respect and the clock ratio is not the one being measured.
if grep -q 'EX0205' "$WORK/build.log" 2>/dev/null; then
    echo "BUILD REJECTED: the toolchain substituted a PLL parameter."
    grep 'EX0205' "$WORK/build.log" | head -4 | sed 's/^/    /'
    echo "  The clocks are not the ones this arm claims to measure. Pick divider"
    echo "  values the part accepts (cdc_pll.v.in lists the ones proven on it)."
    exit 1
fi

# ---- the netlist check ------------------------------------------------------
#
# Every register the experiment depends on, by name. A tool that merged
# write_index into the salt, or inferred the storage as a RAM, or dropped a
# synchronizer stage, leaves a design that still simulates and no longer tests
# what this claims to test.
case "$ARM" in
    # wi_r/ri_r are the stored index registers inside pipe_cdc's g_stored
    # block; arm F has neither, by construction.
    a|b) WANT="write_salt read_salt wi_r ri_r o_wsalt_q src_rsalt_q" ;;
    # Arm F has no index REGISTERS by construction -- that is the point of it --
    # so they are not in the list. The salts still must be there.
    f)   WANT="write_salt read_salt o_wsalt_q src_rsalt_q" ;;
    c)   WANT="write_salt read_salt wi_r ri_r o_wsalt_q src_rsalt_q rs_meta rs_sync ws_meta ws_sync rs2_meta rs2_sync ws2_meta ws2_sync" ;;
    e)   WANT="wbin rbin wgray rgray wgray_meta wgray_sync rgray_meta rgray_sync" ;;
    # Arm G checks the same FIFO registers PLUS the reset handshake, which is
    # the part arm E never had -- and the process cores either side of it, so a
    # crossing left standing with the logic around it optimised away is caught.
    g)   WANT="wbin rbin wgray rgray wgray_meta wgray_sync rgray_meta rgray_sync resetting f_meta f_sync f_applied ack_meta ack_sync o_wsalt_q src_rsalt_q" ;;
esac

VG="$(ls "$WORK"/impl/gwsynthesis/*.vg "$WORK"/impl/pnr/*.vg 2>/dev/null | head -1)"
if [ -z "$VG" ]; then
    echo "WARNING: no post-synthesis netlist found under $WORK/impl -- structure NOT verified."
    echo "         Look for a .vg by hand before trusting this arm's result."
else
    missing=""
    for n in $WANT; do
        grep -q -- "$n" "$VG" || missing="$missing $n"
    done
    if [ -n "$missing" ]; then
        echo "BUILD REJECTED: the netlist is missing:$missing"
        echo "  netlist: $VG"
        echo "  The DUT was optimised into something else; this arm would measure the rewrite."
        exit 1
    fi
    echo "  netlist check: all $(echo $WANT | wc -w) expected registers present in $(basename "$VG")"
fi

if grep -qiE '^ERROR|\bERROR \(' "$WORK/build.log" 2>/dev/null; then
    echo "BUILD FAILED"
    grep -iE '^ERROR|\bERROR \(' "$WORK/build.log" | head -12
    exit 1
fi

echo "BUILD OK: arm $ARM pair $PAIR"
echo "  bitstream: $FS ($(stat -c%s "$FS") bytes)"

# 3. WARNINGS ON SUCCESS. `ODIV_SEL value invalid, replaced by default value 8`
#    is a WARN and nothing else: exit status, bitstream, utilization and timing
#    all stay green while the part runs at a different frequency than the source
#    documents. For this experiment that would silently change the clock ratio.
warns="$(grep -iE '^WARN|WARN \(' "$WORK/build.log" 2>/dev/null)"
if [ -n "$warns" ]; then
    n="$(printf '%s\n' "$warns" | wc -l | tr -d ' ')"
    echo "  $n warning(s):"
    printf '%s\n' "$warns" | head -12 | sed 's/^/    /'
    [ "$n" -gt 12 ] && echo "    ... $((n - 12)) more in $WORK/build.log"
fi

rpt="$WORK/impl/pnr/project.rpt.txt"
[ -f "$rpt" ] || rpt="$(ls "$WORK"/impl/pnr/*.rpt.txt 2>/dev/null | head -1)"
if [ -f "$rpt" ]; then
    grep -iE 'Logic|Register|BSRAM|SSRAM|DSP|I/O Port|Slice|IOLOGIC' "$rpt" |
        grep -iE '[0-9]+/' | sed 's/^/  /'
fi

# 4. From project_tr_content.html. project.tr.html is only a frameset, so a
#    pattern matched against it never matches and every build reports no timing.
tim="$WORK/impl/pnr/project_tr_content.html"
if [ -f "$tim" ]; then
    python - "$tim" <<'PYEND' || true
import html, re, sys

t = open(sys.argv[1], encoding='utf-8', errors='replace').read()
rows = []
for r in re.findall(r'<tr.*?</tr>', t, re.S):
    cells = [html.unescape(re.sub('<[^>]*>', '', c)).strip().replace(' ', '')
             for c in re.findall(r'<t[dh].*?</t[dh]>', r, re.S)]
    rows.append([c for c in cells if c])

# Per-clock Fmax against its constraint. NOTE 'ActualFmax', no space: the cells
# are space-stripped above, so matching 'Actual Fmax' here silently matches
# nothing and every build reports no timing at all.
show = False
for cells in rows:
    line = ' | '.join(cells)
    if 'Constraint' in line and 'ActualFmax' in line:
        show = True
        continue
    if show:
        if len(cells) >= 4 and '(MHz)' in cells[2]:
            want = float(cells[2].split('(')[0])
            got = float(cells[3].split('(')[0])
            flag = '  <-- VIOLATED' if got < want else ''
            print(f"  timing: {cells[1]:<10} {got:>8.3f} MHz against {want:.3f}, "
                  f"{cells[4]} levels{flag}")
        else:
            show = False
for cells in rows:
    if len(cells) == 2 and 'ViolatedEndpoints' in cells[0].replace(' ', ''):
        if cells[1] != '0':
            print(f"  timing: {cells[0]} = {cells[1]}")

# DID THE TOOL TIME THE CROSSING? If set_clock_groups -asynchronous took, there
# are no paths analysed between the two domains. If there are, the tool closed
# the crossing and the arm measured a timed path rather than an asynchronous
# one -- which is the first thing to suspect when arm B comes out clean.
cross = 0
for cells in rows:
    if len(cells) >= 7 and ':[' in cells[4] and ':[' in cells[5]:
        if cells[4].split(':')[0] != cells[5].split(':')[0]:
            cross += 1
if cross:
    print(f"  CROSSING: {cross} path(s) analysed BETWEEN the two clock domains.")
    print( "            The tool is timing the crossing. A clean arm B would mean")
    print( "            nothing -- fix arm.sdc before believing any result.")
else:
    print("  crossing: no inter-domain paths analysed (the domains are declared asynchronous)")
PYEND
fi
echo "  full log: $WORK/build.log"

if [ "$FLASH" = "1" ]; then
    if [ -x "$PROGRAMMER" ]; then
        echo "flashing (SRAM) ..."
        # Mode 2 is SRAM write: nothing is written to the board's flash, so a
        # power cycle returns it to whatever was there before.
        "$PROGRAMMER" --device GW1NR-9C --run 2 --fsFile "$(cygpath -w "$FS" 2>/dev/null || echo "$FS")"
    elif command -v openFPGALoader >/dev/null 2>&1; then
        openFPGALoader -b tangnano9k "$FS"
    else
        echo "no programmer found (looked for $PROGRAMMER, then openFPGALoader on PATH)" >&2
        exit 1
    fi
    echo "loaded. Now open a serial terminal at 115200 8N1; one line a second:"
    echo "    <arm> rx1 err1 idle1_max rx2 err2 idle2_max cyc     (all hex)"
fi
