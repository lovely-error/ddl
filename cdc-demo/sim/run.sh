#!/usr/bin/env bash
# Throughput across the crossing, in Questa. No board needed.
#
#   bash cdc-demo/sim/run.sh
#
# Measures arm C's claim, which is a counting argument about a credit loop and
# so is exact in RTL. It does NOT measure arm B's: this simulator samples
# old-or-new and models no metastability, so an unsynchronized crossing is clean
# here by construction. Read the throughput column, not the error column.
#
# Swept over three clock ratios, because the depth a crossing needs is a
# function of the ratio and one data point would not show that.
set -u
QUESTA="${QUESTA:-/e/Quartus/questa_fse/win64}"
HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$HERE/work"

[ -x "$QUESTA/vlog.exe" ] || { echo "Questa not found at $QUESTA (set QUESTA=<path>)" >&2; exit 1; }

rm -rf "$WORK"; mkdir -p "$WORK"
cd "$WORK"

"$QUESTA/vlib.exe" work >/dev/null || exit 1
"$QUESTA/vlog.exe" -quiet -sv "$HERE/../../lib/ddl_cdc_fifo.v" "$HERE/../pipe_cdc.v"     "$HERE/../cdc_fifo.v" "$HERE/tb_cdc_demo.sv" "$HERE/tb_cdc_lib.sv" || exit 1

# The shipped module first: is it the same design that ran on the board, and do
# its parameters work? `cdc_fifo.v` is the frozen artifact arm E used; if the two
# ever disagree, the hardware evidence stops applying to what ships.
echo "================================================================"
echo "  lib/ddl_cdc_fifo.v -- equivalence to the measured artifact, and parameters"
"$QUESTA/vsim.exe" -quiet -c -do "run -all; quit -f" tb_cdc_lib 2>&1 |
    sed 's/^# //' | grep -E "EQUIVALENCE|OK:|FAIL|PARAMETERS|shape|WIDTH|ceiling is"

run_one() {
    echo "================================================================"
    echo "  $3"
    "$QUESTA/vsim.exe" -quiet -c -gPERIOD_A=$1 -gPERIOD_B=$2 \
        -do "run -all; quit -f" tb_cdc_demo 2>&1 |
        grep -E "clock A|DUT |pipe_cdc|cdc_fifo|ceiling|FAIL" | sed 's/^# //'
}

run_one 12.346 9.259  "81 MHz source -> 108 MHz sink   (build.sh pair 0, 4:3)"
run_one 12.346 10.582 "81 MHz source -> 94.5 MHz sink  (build.sh pair 1, 7:6)"
run_one 14.815 9.259  "67.5 MHz source -> 108 MHz sink (build.sh pair 2, 8:5)"
run_one 9.259  9.259  "108 MHz both                    (equal clocks, the worst case)"
