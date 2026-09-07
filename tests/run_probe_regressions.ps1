param(
    [string]$SimTool = 'E:/Quartus/questa_fse/win64'
)
$ErrorActionPreference = 'Stop'
$compilerRoot = Split-Path -Parent $PSScriptRoot
$outputDirectory = Join-Path $compilerRoot 'target/probe-regressions'
New-Item -ItemType Directory -Force $outputDirectory | Out-Null
Push-Location $compilerRoot
try {
    & cargo build --locked
    if ($LASTEXITCODE -ne 0) { throw 'Compiler build failed' }
    $ddlBinary = Join-Path $compilerRoot 'target/debug/ddl.exe'
    foreach ($probeSource in Get-ChildItem (Join-Path $PSScriptRoot 'probes/*.ddl')) {
        $output = Join-Path $outputDirectory ($probeSource.BaseName + '.v')
        & $ddlBinary build $probeSource.FullName -o $output
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
        foreach ($bench in @('tb_fixed', 'tb_adversarial', 'tb_sequence')) {
            & (Join-Path $SimTool 'vlog.exe') -quiet -work work (Join-Path $PSScriptRoot "probes/$bench.sv") *> "$bench.vlog.log"
            if ($LASTEXITCODE -ne 0) { throw "Testbench compilation failed: $bench" }
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
