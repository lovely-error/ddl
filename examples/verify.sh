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
# Used for the compiler's `-I`, and so it lands in the generated file's
# "Regenerate with:" banner. Relative on purpose: a banner naming
# /e/Code/... is a command only this machine can run.
K2G_INC="${K2G_INC:-../KAMASUTRA2G/rtl}"
QUESTA="${QUESTA:-/e/Quartus/questa_fse/win64}"
GW_SH="${GW_SH:-/c/Gowin/Gowin_V1.9.12.02_SP2_x64/IDE/bin/gw_sh.exe}"
WORK="${WORK:-$DDL_ROOT/target/verify}"
RTL="$K2G/rtl"

# bram_lookup has no SystemVerilog counterpart and no equivalence testbench.
# It is here for the RAM primitive count alone: `bram` picks a physical
# primitive, and the netlist is the only thing that can say whether it got one.
#
# rf_lvt and rf_lvt_proc are checked against THEMSELVES built the other way.
# `--lvt-bram` is a second way to build one description, so the two sides come
# from one source: what is under test is whether banks and a live value table
# hold the same memory as the multi-write cell they stand in for, and a
# hand-written reference would only be a second opinion about what the
# description meant. Their testbenches carry a behavioural model too, because
# two builds of one wrong idea agree with each other perfectly.
#
# BOTH, because a sequence and a process reach the ports through different code
# and come out a different shape: a process muxes every read onto one port, so
# its banks have one replica where the pipeline's have one per read. Proving
# either says nothing about the other.
VARIANT_MODULES=(rf_lvt rf_lvt_proc)
ALL_MODULES=(k2g_shift k2g_alu k2g_decode k2g_xstage k3g_stage fsm_adder mul3 bram_lookup rf_lvt rf_lvt_proc)
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
  src="examples/$m.ddl"
  gen="$DDL_ROOT/examples/$m.v"
  tb="$DDL_ROOT/examples/tb_${m}_equiv.sv"

  # Each module names its own dependencies with `import`, so the whole build
  # is the one file. k2g_pkg.ddl is generated from the emulator by
  # emu/src/ddl_gen.rs -- the same source as k2g_pkg.sv -- and lives in the
  # consumer's tree, which is what -I is for.
  #
  # Run from DDL_ROOT with relative paths: the command ends up verbatim in the
  # generated file's banner, and it has to be one anybody can run.
  [ -f "$DDL_ROOT/$src" ] || { fail "no $src"; continue; }
  # `-I` only when the module actually imports something. The command ends up
  # verbatim in the banner, and a search path that is never consulted is a
  # command that reads as though it needs a tree it does not.
  inc=()
  grep -q '^import ' "$DDL_ROOT/$src" && inc=(-I "$K2G_INC")
  ( cd "$DDL_ROOT" && "$DDL_BIN" build "$src" "${inc[@]}" -o "examples/$m.v" )       || { fail "ddl build"; continue; }
  note "generated $(basename "$gen")"

  # A SECOND build, with the boundary left bare.
  #
  # What is under test here is the LOWERING: the generated logic against the
  # hand-written module it replaces, which speaks the salt protocol because
  # both ends of that wire were written by hand. The checked-in file above
  # presents a FIFO instead -- right for someone instantiating it, wrong for
  # this comparison, which would otherwise measure two adapters and a wrapper
  # against a module that has neither.
  #
  # The adapters and the wrapper are proven separately, and by simulation:
  # tests/adapters.rs drives them cycle by cycle against the FIFO contract.
  bare="$WORK/$m/bare.v"
  mkdir -p "$WORK/$m"
  ( cd "$DDL_ROOT" && "$DDL_BIN" build "$src" "${inc[@]}" --bare-export "$m" -o "$bare" ) \
      || { fail "ddl build --bare-export"; continue; }

  # A module with a *_ref.sv beside it is checked against that hand-written
  # reference rather than against a K2G module: for k3g_stage what is under
  # test is the GENERATED handshake, so the reference has to be the handshake
  # a person would otherwise have written.
  ref_sv="$RTL/$m.sv"
  ref_srcs=("$RTL/k2g_pkg.sv" "$RTL/k2g_types.svh" "$RTL/$m.sv")
  standalone=0
  variant=0
  parts=()
  # Built from this same .ddl with a different flag rather than found on disk.
  # `ref_sv` is pointed at the generated file only so the "no reference"
  # branch below does not claim there is none.
  if printf '%s\n' "${VARIANT_MODULES[@]}" | grep -qx "$m"; then
    variant=1
    standalone=1
    ref_sv="$gen"
    ref_srcs=()
  elif [ -f "$DDL_ROOT/examples/${m}_ref.sv" ]; then
    ref_sv="$DDL_ROOT/examples/${m}_ref.sv"
    ref_srcs=("$ref_sv")
    standalone=1
  fi
  # k2g_xstage's reference is not hand-written logic: it instantiates the
  # unmodified regfile, ALU and shifter and wires them the way k2g_core.sv
  # does. They compile with it, and synthesize with it.
  if [ "$m" = "k2g_xstage" ]; then
    parts=("$RTL/k2g_pkg.sv" "$RTL/k2g_types.svh" "$RTL/k2g_regfile.sv" "$RTL/k2g_alu.sv" "$RTL/k2g_shift.sv")
    ref_srcs=("${parts[@]}" "$ref_sv")
  fi

  # A module with no counterpart anywhere is synthesis-only. bram_lookup is
  # the case: there is nothing to prove it equivalent TO, and the reason it
  # exists is the primitive its netlist contains.
  no_reference=0
  if [ ! -f "$ref_sv" ]; then
    no_reference=1
    note "no reference: synthesis only"
  fi

  eq="$WORK/$m/equiv"
  mkdir -p "$eq"
  if [ "$variant" = "1" ]; then
    ( cd "$DDL_ROOT" && "$DDL_BIN" build --lvt-bram "$src" --bare-export "$m" -o "$eq/raw_lvt.v" ) \
        || { fail "ddl build --lvt-bram"; continue; }
    # Renamed so both builds can be instantiated in one testbench.
    sed "s/^module $m (/module ${m}_lvt (/" "$eq/raw_lvt.v" > "$eq/${m}_lvt.v"
    rm -f "$eq/raw_lvt.v"
    ref_srcs=("$eq/${m}_lvt.v")
    note "generated ${m}_lvt.v with --lvt-bram"
  fi
  if [ "$standalone" = "1" ]; then
    cp "$bare" "$eq/${m}_ddl.v"
  else
    # Renamed so both can be instantiated in one testbench.
    sed "s/module $m (/module ${m}_ddl (/" "$bare" > "$eq/${m}_ddl.v"
  fi

  if [ "$no_reference" = "1" ]; then
    note "SKIP equivalence: nothing to compare against"
  elif [ ! -x "$QUESTA/vlog.exe" ]; then
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
      "$QUESTA/vlog.exe" -sv -quiet +define+SIMULATION "+incdir+$RTL_W" \
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
  # The bare build again: an area comparison against a module with salt ports
  # has to be of the thing that has salt ports.
  cp "$bare" "$syn/ddl/$m.v"
  cat > "$syn/ddl/syn.tcl" <<TCL
set_device -name GW1NR-9C GW1NR-LV9QN88PC6/I5
add_file -type verilog {$m.v}
set_option -top_module $m
set_option -verilog_std sysv2017
set_option -include_path {.}
run syn
TCL
  if [ "$no_reference" = "1" ]; then
    : # nothing on the other side to synthesize
  elif [ ${#parts[@]} -gt 0 ]; then
    cp "${parts[@]}" "$syn/ref/"
    cp "$ref_sv" "$syn/ref/$m.sv"
    ref_top="${m}_ref"
    pkg_line="add_file -type verilog {k2g_pkg.sv}
add_file -type verilog {k2g_regfile.sv}
add_file -type verilog {k2g_alu.sv}
add_file -type verilog {k2g_shift.sv}"
  elif [ "$variant" = "1" ]; then
    # The measurement the flag exists for: the same description, synthesized
    # both ways, so the RAM primitive counts below say whether building the
    # memory out of one-write blocks got a block RAM back.
    cp "$eq/${m}_lvt.v" "$syn/ref/$m.sv"
    ref_top="${m}_lvt"
    pkg_line=""
  elif [ "$standalone" = "1" ]; then
    cp "$ref_sv" "$syn/ref/$m.sv"
    ref_top="${m}_ref"
    pkg_line=""
  else
    cp "$RTL/k2g_pkg.sv" "$RTL/k2g_types.svh" "$RTL/$m.sv" "$syn/ref/"
    ref_top="$m"
    pkg_line="add_file -type verilog {k2g_pkg.sv}"
  fi
  if [ "$no_reference" != "1" ]; then
  cat > "$syn/ref/syn.tcl" <<TCL
set_device -name GW1NR-9C GW1NR-LV9QN88PC6/I5
$pkg_line
add_file -type verilog {$m.sv}
set_option -top_module $ref_top
set_option -verilog_std sysv2017
set_option -include_path {.}
run syn
TCL
  fi

  count_cells() {
    # THE NETLIST IS THE VERDICT, not gw_sh's exit code: it exits 1 on runs
    # that produced a perfectly good result.
    local vg="$1/impl/gwsynthesis/project.vg"
    [ -f "$vg" ] || { echo "-1"; return; }
    grep -cE '^\s*(LUT[0-9]|ALU|MUX2_LUT[0-9]|DFF[A-Z]*|RAM16[A-Z0-9]*|SDPB?|DPB?|SP|ROM)\b' "$vg"
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
  sides=(ddl ref)
  [ "$no_reference" = "1" ] && sides=(ddl)
  for d in "${sides[@]}"; do
    attempt=1
    until synth_once "$syn/$d"; do
      # Only the silent crash is worth another go. A run that produced a log
      # and objected in it will object again, and retrying buries the reason
      # under two more minutes of the same.
      if [ -s "$syn/$d/syn.log" ]; then break; fi
      attempt=$((attempt + 1))
      if [ "$attempt" -gt 3 ]; then break; fi
      note "retrying $d synthesis (gw_sh produced nothing)"
    done
  done
  ddl_cells=$(count_cells "$syn/ddl")
  ref_cells=$(count_cells "$syn/ref")

  count_rams() {
    local vg="$1/impl/gwsynthesis/project.vg"
    [ -f "$vg" ] || { echo 0; return; }
    grep -cE '^\s*(RAM16[A-Z0-9]*|SDPB?|DPB?|SP|ROM)\b' "$vg"
  }

  # WHETHER THE DEFAULT BUILD SYNTHESIZES AT ALL IS THE MEASUREMENT, so it is
  # reported rather than required. Two write ports is a shape this device has
  # no cell for, and what GowinSynthesis does instead is fall back to
  # flip-flops -- which for a table of any size does not fit. `--lvt-bram` is
  # the way back to a RAM primitive, so that is the side this asserts on.
  if [ "$variant" = "1" ]; then
    if [ "$ddl_cells" -lt 0 ]; then
      why=$(grep -oE 'ERROR \([A-Z0-9]+\) : .*' "$syn/ddl/syn.log" 2>/dev/null | head -1)
      note "default build: no netlist -- ${why:-gw_sh produced nothing}"
    else
      note "default build: $ddl_cells cells, $(count_rams "$syn/ddl") RAM primitives"
    fi
    if [ "$ref_cells" -lt 0 ]; then
      why=$(grep -oE 'ERROR \([A-Z0-9]+\) : .*' "$syn/ref/syn.log" 2>/dev/null | head -1)
      fail "--lvt-bram build: no netlist -- ${why:-gw_sh produced nothing}"
    else
      lvt_rams=$(count_rams "$syn/ref")
      note "--lvt-bram build: $ref_cells cells, $lvt_rams RAM primitives"
      if [ "$lvt_rams" -eq 0 ]; then
        fail "--lvt-bram inferred no RAM primitive, which is the only reason it exists"
      fi
    fi
    continue
  fi

  missing_netlist=0
  [ "$ddl_cells" -lt 0 ] && missing_netlist=1
  [ "$no_reference" != "1" ] && [ "$ref_cells" -lt 0 ] && missing_netlist=1
  if [ "$missing_netlist" = "1" ]; then
    fail "synthesis produced no netlist"
  elif [ "$ddl_cells" -lt 3 ]; then
    # An anti-vacuous guard, as rtl/sv-probe/run_pkg_check.sh has: a design
    # that optimized away proves nothing.
    fail "DDL design optimized away ($ddl_cells cells)"
  else
    if [ "$no_reference" = "1" ]; then
      note "area: ddl $ddl_cells cells (nothing to compare against)"
    else
      note "$(awk -v a="$ddl_cells" -v b="$ref_cells" \
          'BEGIN { printf "area: ddl %d, ref %d, delta %+d (%+.1f%%)", a, b, a-b, 100*(a-b)/b }')"
    fi
    # GowinSynthesis emits SP00018 "error bus name set" once per BIT of some
    # named intermediate buses its optimizer eliminates. Measured spurious: the
    # netlist is produced, the cell count is unchanged, and equivalence passes.
    # Reported rather than failed on, because they are not this compiler's bug
    # to fix -- but reported every run, because this toolchain's real failures
    # are documented as easy to miss and must not hide among these.
    ddl_rams=$(count_rams "$syn/ddl")
    ref_rams=$(count_rams "$syn/ref")
    if [ "$ddl_rams" -gt 0 ] || [ "$ref_rams" -gt 0 ]; then
      note "RAM primitives: ddl $ddl_rams, ref $ref_rams"
      if [ "$ref_rams" -gt 0 ] && [ "$ddl_rams" -eq 0 ]; then
        fail "the reference inferred RAM and the DDL version did not"
      fi
    fi
    # `bram` picks a physical primitive, and the netlist is the only thing that
    # can say whether it got one. A block RAM that infers no RAM primitive is
    # distributed RAM wearing the wrong label, which is what this module exists
    # to catch and what nothing else can.
    if [ "$m" = "bram_lookup" ] && [ "$ddl_rams" -eq 0 ]; then
      fail "bram_lookup inferred no RAM primitive: it is not a block RAM"
    fi
    gw_errors=$(grep -c 'ERROR' "$syn/ddl/syn.log" 2>/dev/null)
    ref_errors=$(grep -c 'ERROR' "$syn/ref/syn.log" 2>/dev/null)
    [ -z "$gw_errors" ] && gw_errors=0
    [ -z "$ref_errors" ] && ref_errors=0
    note "gw_sh diagnostics: ddl $gw_errors, ref $ref_errors (SP00018 is spurious)"
  fi
done

echo
if [ "$failures" -eq 0 ]; then echo "OK"; else echo "$failures failure(s)"; exit 1; fi
