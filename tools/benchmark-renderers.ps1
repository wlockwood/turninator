[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Runner,
    [Parameter(Mandatory)][string]$PackageDirectory,
    [Parameter(Mandatory)][string]$OutputRoot,
    [ValidateSet('wgpu','filament','luxcore','blender_eevee','blender_cycles')]
    [string[]]$Engines = @('wgpu','filament','luxcore','blender_eevee','blender_cycles'),
    [ValidateSet('opaque','beads','stepped-cutaway','resin')]
    [string[]]$Scenes = @('opaque','beads','stepped-cutaway','resin'),
    [ValidateSet('animation','still')][string[]]$Kinds = @('animation','still')
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$runnerPath = (Resolve-Path -LiteralPath $Runner).Path
$packagePath = (Resolve-Path -LiteralPath $PackageDirectory).Path
$output = [IO.Path]::GetFullPath($OutputRoot)
if (Test-Path -LiteralPath $output) { throw 'Choose a new benchmark output directory; evidence is never overwritten.' }
New-Item -ItemType Directory -Path $output | Out-Null
$runtime = Join-Path $output 'runtime'
New-Item -ItemType Directory -Path $runtime | Out-Null
Copy-Item -LiteralPath $runnerPath -Destination (Join-Path $runtime 'renderer-benchmark.exe')
Copy-Item -LiteralPath (Join-Path $packagePath 'renderers') -Destination $runtime -Recurse
Copy-Item -LiteralPath (Join-Path $workspace 'renderer') -Destination $runtime -Recurse
$provenance = [ordered]@{ recipe='matched-renderers-v1'; runner_sha256=(Get-FileHash -LiteralPath $runnerPath).Hash.ToLowerInvariant(); files=@() }
foreach ($file in (Get-ChildItem -LiteralPath $runtime -Recurse -File)) {
    $provenance.files += @{path=[IO.Path]::GetRelativePath($runtime,$file.FullName); sha256=(Get-FileHash -LiteralPath $file.FullName).Hash.ToLowerInvariant()}
}
$provenance | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'provenance.json') -Encoding utf8
$prior = @{}
foreach ($name in @('TN_BENCH_OUTPUT','TN_BENCH_BACKEND','TN_BENCH_SCENE','TN_BENCH_OUTPUT_KIND')) {
    $prior[$name] = [Environment]::GetEnvironmentVariable($name,'Process')
}
try {
    foreach ($engine in $Engines) {
        foreach ($scene in $Scenes) {
            foreach ($kind in $Kinds) {
                $case = Join-Path $output "$engine-$scene-$kind"
                New-Item -ItemType Directory -Path $case | Out-Null
                $env:TN_BENCH_OUTPUT = $case
                $env:TN_BENCH_BACKEND = $engine
                $env:TN_BENCH_SCENE = $scene
                $env:TN_BENCH_OUTPUT_KIND = $kind
                Write-Output "Starting $engine / $scene / $kind at $(Get-Date -Format o)"
                $auditArgs = @((Join-Path $PSScriptRoot 'process_audit.py'), '--cwd', $output,
                    '--output', (Join-Path $case 'process-audit.json'), '--timeout', '1800')
                if ($kind -eq 'still' -and $engine -in @('wgpu','filament','luxcore')) { $auditArgs += '--require-no-children' }
                $auditArgs += @('--', (Join-Path $runtime 'renderer-benchmark.exe'), '--exact', 'matched_production_case', '--ignored', '--nocapture')
                & python @auditArgs *> (Join-Path $case 'run.log')
                if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath (Join-Path $case 'result.json'))) {
                    throw "Benchmark failed: $engine / $scene / $kind; preserve $case for diagnosis."
                }
                $result = Get-Content -LiteralPath (Join-Path $case 'result.json') -Raw | ConvertFrom-Json
                Write-Output "Completed $engine / $scene / $kind in $($result.end_to_end_seconds) seconds"
            }
        }
    }
} finally {
    foreach ($name in $prior.Keys) { [Environment]::SetEnvironmentVariable($name,$prior[$name],'Process') }
}
Write-Output "Matched benchmark evidence: $output"
