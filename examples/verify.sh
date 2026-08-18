#!/usr/bin/env bash
#
# Proves each DDL port against the hand-written SystemVerilog it replaces, two
# ways:
#
#   1. Equivalence. Both modules are instantiated side by side and driven with
#      the same stimulus -- exhaustive over the small fields, random over the
#      wide ones. Any mismatch fails.
#   2. Area. Both are synthesized for the GW1NR-9C and their primitive counts
#      compared. The project rule is "synthesize each module as it is written,
#      not at integration" (docs/gowin-sv-support.md); a functionally correct
#      module that is materially larger is not yet a replacement.
#
# Git Bash only, and Questa is licence-nodelocked -- one at a time.
#
# Usage: bash examples/verify.sh [module ...]     (default: all)
set -uo pipefail

DDL_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
K2G="${K2G:-/e/Code/KAMASUTRA2G}"
QUESTA="${QUESTA:-/e/Quartus/questa_fse/win64}"
GW_SH="${GW_SH:-/c/Gowin/Gowin_V1.9.12.02_SP2_x64/IDE/bin/gw_sh.exe}"
WORK="${WORK:-$DDL_ROOT/target/verify}"
RTL="$K2G/rtl"

ALL_MODULES=(k2g_shift k2g_alu k2g_decode k3g_stage fsm_adder mul3)
MODULES=("$@")
[ ${#MODULES[@]} -eq 0 ] && MODULES=("${ALL_MODULES[@]}")

failures=0
note() { echo "  $*"; }
fail() { echo "  FAIL: $*" >&2; failures=$((failures + 1)); }

[ -d "$RTL" ] || { echo "no RTL at $RTL (set K2G=<path>)" >&2; exit 1; }

echo "== building the compiler =="
cargo build --quiet --manifest-path "$DDL_ROOT/Cargo.toml" 2>/dev/null \
    || { echo "cargo build failed" >&2; exit 1; }
DDL_BIN="$DDL_ROOT/target/debug/ddl"
[ -x "$DDL_BIN" ] || DDL_BIN="$DDL_BIN.exe"

rm -rf "$WORK"

for m in "${MODULES[@]}"; do
  echo
  echo "== $m =="
  src="$DDL_ROOT/examples/$m.ddl"
  gen="$DDL_ROOT/examples/$m.v"
  tb="$DDL_ROOT/examples/tb_${m}_equiv.sv"

  # DDL has no import mechanism yet, so a module needing the shared types is
  # compiled from a concatenation. k2g_pkg.ddl is generated from the emulator
  # by emu/src/ddl_gen.rs -- the same source as k2g_pkg.sv.
  case "$m" in
    k2g_decode)
      mkdir -p "$WORK/$m"
      src="$WORK/$m/src.ddl"
      cat "$K2G/rtl/k2g_pkg.ddl"           "$DDL_ROOT/examples/k2g_types.ddl"           "$DDL_ROOT/examples/k2g_decode.ddl" > "$src"
      ;;
  esac

  [ -f "$src" ] || { fail "no $src"; continue; }
  "$DDL_BIN" build "$src" -o "$gen" || { fail "ddl build"; continue; }
  note "generated $(basename "$gen")"

  # A module with a *_ref.sv beside it is checked against that hand-written
  # reference rather than against a K2G module: for k3g_stage what is under
  # test is the GENERATED handshake, so the reference has to be the handshake
  # a person would otherwise have written.
  ref_sv="$RTL/$m.sv"
  ref_srcs=("$RTL/k2g_pkg.sv" "$RTL/k2g_types.svh" "$RTL/$m.sv")
  standalone=0
  if [ -f "$DDL_ROOT/examples/${m}_ref.sv" ]; then
    ref_sv="$DDL_ROOT/examples/${m}_ref.sv"
    ref_srcs=("$ref_sv")
    standalone=1
  fi

  eq="$WORK/$m/equiv"
  mkdir -p "$eq"
  if [ "$standalone" = "1" ]; then
    cp "$gen" "$eq/${m}_ddl.v"
  else
    # Renamed so both can be instantiated in one testbench.
    sed "s/module $m (/module ${m}_ddl (/" "$gen" > "$eq/${m}_ddl.v"
  fi

  if [ ! -x "$QUESTA/vlog.exe" ]; then
    note "SKIP equivalence: no Questa at $QUESTA"
  elif [ ! -f "$tb" ]; then
    fail "no testbench $tb"
  else
    cp "$tb" "$eq/"
    (
      cd "$eq" || exit 1
      RTL_W="$(cygpath -m "$RTL")"
      "$QUESTA/vlib.exe" work >/dev/null 2>&1
      srcs=()
      for f in "${ref_srcs[@]}"; do srcs+=("$(cygpath -m "$f")"); done
      "$QUESTA/vlog.exe" -sv -quiet "+incdir+$RTL_W" \
          "${srcs[@]}" \
          "${m}_ddl.v" "$(basename "$tb")" > vlog.log 2>&1 \
          || { tail -20 vlog.log; exit 1; }
      "$QUESTA/vsim.exe" -c -quiet \
          -do "onerror {quit -code 1}; run -all; quit -f" \
          "tb_${m}_equiv" > vsim.log 2>&1
      # The same triple rule as rtl/sim/run.sh: a testbench must not pass by
      # saying nothing.
      grep -q "TB_PASS" vsim.log || { grep -E 'MISMATCH|TB_FAIL' vsim.log | head -5; exit 1; }
      grep -qE '^# \*\* (Error|Fatal)' vsim.log && { tail -20 vsim.log; exit 1; }
      grep -oE 'TB_PASS.*' vsim.log
    ) && note "equivalence OK" || fail "equivalence"
  fi

  if [ ! -x "$GW_SH" ]; then
    note "SKIP area: no gw_sh at $GW_SH"
    continue
  fi

  syn="$WORK/$m/syn"
  mkdir -p "$syn/ddl" "$syn/ref"
  cp "$gen" "$syn/ddl/"
  cat > "$syn/ddl/syn.tcl" <<TCL
set_device -name GW1NR-9C GW1NR-LV9QN88PC6/I5
add_file -type verilog {$m.v}
set_option -top_module $m
set_option -verilog_std sysv2017
set_option -include_path {.}
run syn
TCL
  if [ "$standalone" = "1" ]; then
    cp "$ref_sv" "$syn/ref/$m.sv"
    ref_top="${m}_ref"
    pkg_line=""
  else
    cp "$RTL/k2g_pkg.sv" "$RTL/k2g_types.svh" "$RTL/$m.sv" "$syn/ref/"
    ref_top="$m"
    pkg_line="add_file -type verilog {k2g_pkg.sv}"
  fi
  cat > "$syn/ref/syn.tcl" <<TCL
set_device -name GW1NR-9C GW1NR-LV9QN88PC6/I5
$pkg_line
add_file -type verilog {$m.sv}
set_option -top_module $ref_top
set_option -verilog_std sysv2017
set_option -include_path {.}
run syn
TCL

  count_cells() {
    # THE NETLIST IS THE VERDICT, not gw_sh's exit code: it exits 1 on runs
    # that produced a perfectly good result.
    local vg="$1/impl/gwsynthesis/project.vg"
    [ -f "$vg" ] || { echo "-1"; return; }
    grep -cE '^\s*(LUT[0-9]|ALU|MUX2_LUT[0-9]|DFF[A-Z]*)\b' "$vg"
  }

  # gw_sh intermittently exits having written nothing at all: no netlist, no
  # log, not even its startup banner. docs/bring-up.md records the same
  # signature ("Silent crashes are common"), and here it shows up only when
  # two synthesis runs follow one another closely. Retry once before believing
  # it -- the netlist is the verdict, so ask again before declaring none.
  synth_once() {
    local dir="$1"
    rm -rf "$dir/impl"
    ( cd "$dir" && "$GW_SH" syn.tcl > syn.log 2>&1 )
    [ -f "$dir/impl/gwsynthesis/project.vg" ]
  }
  for d in ddl ref; do
    attempt=1
    until synth_once "$syn/$d"; do
      attempt=$((attempt + 1))
      if [ "$attempt" -gt 3 ]; then break; fi
      note "retrying $d synthesis (gw_sh produced nothing)"
    done
  done
  ddl_cells=$(count_cells "$syn/ddl")
  ref_cells=$(count_cells "$syn/ref")

  if [ "$ddl_cells" -lt 0 ] || [ "$ref_cells" -lt 0 ]; then
    fail "synthesis produced no netlist"
  elif [ "$ddl_cells" -lt 3 ]; then
    # An anti-vacuous guard, as rtl/sv-probe/run_pkg_check.sh has: a design
    # that optimized away proves nothing.
    fail "DDL design optimized away ($ddl_cells cells)"
  else
    note "$(awk -v a="$ddl_cells" -v b="$ref_cells" \
        'BEGIN { printf "area: ddl %d, ref %d, delta %+d (%+.1f%%)", a, b, a-b, 100*(a-b)/b }')"
    # GowinSynthesis emits SP00018 "error bus name set" once per BIT of some
    # named intermediate buses its optimizer eliminates. Measured spurious: the
    # netlist is produced, the cell count is unchanged, and equivalence passes.
    # Reported rather than failed on, because they are not this compiler's bug
    # to fix -- but reported every run, because this toolchain's real failures
    # are documented as easy to miss and must not hide among these.
    gw_errors=$(grep -c 'ERROR' "$syn/ddl/syn.log" 2>/dev/null)
    ref_errors=$(grep -c 'ERROR' "$syn/ref/syn.log" 2>/dev/null)
    [ -z "$gw_errors" ] && gw_errors=0
    [ -z "$ref_errors" ] && ref_errors=0
    note "gw_sh diagnostics: ddl $gw_errors, ref $ref_errors (SP00018 is spurious)"
  fi
done

echo
if [ "$failures" -eq 0 ]; then echo "OK"; else echo "$failures failure(s)"; exit 1; fi
