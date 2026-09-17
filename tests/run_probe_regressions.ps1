param(
    [string]$SimTool = 'E:/Quartus/questa_fse/win64'
)
$ErrorActionPreference = 'Stop'
$compilerRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $compilerRoot 'target/probe-regressions'
New-Item -ItemType Directory -Force $outputDirectory | Out-Null

# Which modules each probe presents, spelled out.
#
# `ddl build` refuses to guess when a file has several roots, and this used to
# call it with no selection at all -- so the run stopped on the first
# multi-root probe and never reached a simulator. Listing them here rather than
# defaulting is deliberate: a probe that grows a module fails this script with
# a name to add, instead of quietly dropping the module from the run.
#
# `--bare-export` and not `--export`: the benches connect the raw salt
# interface, so wrapping these in the FIFO adapter would change what is tested.
$exportManifest = @{
    'adversarial'   = @('conditional_forward','exclusive_match','exclusive_port','for_read2','joined_if','store_and_load','store_before_read')
    'arm_scopes'    = @('arm_scopes')
    'backend_edges' = @('collision_graph','name_collision','scalar_dynamic','scalar_expr','scalar_sext','scalar_signed','scalar_slice','scalar_trunc')
    'communication' = @('polling_drain','polling_forward','polling_once','polling_peek')
    'lifetimes'     = @('lifetimes','received_local')
    'nested_mem'    = @('nm_lane','nm_two','nm_field','nm_branch','nm_seq')
    'p1'            = @('p1')
    'p10'           = @('p10')
    'p11'           = @('p11')
    'p3'            = @('p3')
    'p4'            = @('p4')
    'p5'            = @('p5a','p5b','p5c')
    'p6'            = @('pack_small','unpack')
    'p7'            = @('p7a','p7b')
    'p8'            = @('p8')
    'p9'            = @('p9')
    'sequences'     = @('seq_address','seq_assert','seq_early','seq_join','seq_literal','seq_optional','seq_blocked_early','seq_offer','seq_peek_drop','seq_read_first','seq_route','seq_scope','seq_side')
}

Push-Location $compilerRoot
try {
    & cargo build --locked
    if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
    $ddlBinary = Join-Path $compilerRoot 'target/debug/ddl.exe'
    foreach ($probeSource in Get-ChildItem (Join-Path $PSScriptRoot 'probes/*.ddl')) {
        $name = $probeSource.BaseName
        if (-not $exportManifest.ContainsKey($name)) {
            throw "No export manifest for probe '$name'; add it to `$exportManifest in this script"
        }
        $output = Join-Path $outputDirectory ($name + '.v')
        & $ddlBinary build $probeSource.FullName --bare-export ($exportManifest[$name] -join ',') -o $output
        if ($LASTEXITCODE -ne 0) { throw "DDL compilation failed: $($probeSource.Name)" }
    }
    Push-Location $outputDirectory
    try {
        & (Join-Path $SimTool 'vlib.exe') work *> vlib.log
        if ($LASTEXITCODE -ne 0) { throw 'vlib failed' }
        # .v is compiled in Verilog mode; only the testbench uses .sv.
        foreach ($verilogSource in Get-ChildItem *.v) {
            & (Join-Path $SimTool 'vlog.exe') -quiet -work work +define+SIMULATION $verilogSource.Name *> ($verilogSource.BaseName + '.vlog.log')
            if ($LASTEXITCODE -ne 0) { throw "Verilog rejected: $($verilogSource.Name)" }
        }
        foreach ($bench in @('tb_fixed', 'tb_adversarial', 'tb_sequence', 'tb_nested_mem')) {
            $benchLog = "$bench.vlog.log"
            & (Join-Path $SimTool 'vlog.exe') -quiet -work work (Join-Path $PSScriptRoot "probes/$bench.sv") *> $benchLog
            if ($LASTEXITCODE -ne 0) { throw "Testbench compilation failed: $bench" }
            # An interface that does not line up is reported as a WARNING and
            # then simulated: a missing port reads as `z` and the bench fails
            # much later on a value nothing ever drove, or worse, passes. The
            # mismatch is the finding, so it fails here.
            $elaboration = Get-Content -Raw $benchLog
            if ($elaboration -match '\*\* Warning.*(too few|too many|port|width)') {
                throw "Interface mismatch connecting $bench; inspect $outputDirectory/$benchLog"
            }
            $log = if ($bench -eq 'tb_fixed') { 'vsim.log' } else { "$bench.vsim.log" }
            & (Join-Path $SimTool 'vsim.exe') -c -quiet -voptargs=+acc -do 'run -all; quit -f' "work.$bench" *> $log
            $simulationExit = $LASTEXITCODE
            $transcript = Get-Content -Raw $log
            if ($simulationExit -ne 0 -or $transcript -notmatch 'TB_PASS:' -or $transcript -match '\*\* (Error|Fatal)') {
                throw "Probe simulation failed; inspect $outputDirectory/$log"
            }
            $transcript -split "`n" | Where-Object { $_ -match 'TB_PASS:' }
        }
    } finally { Pop-Location }
} finally { Pop-Location }
