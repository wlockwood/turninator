<#! This script verifies pinned build inputs or runs CPU fixture contract tests.
    It does not run GPU probes, download SDKs, or claim production qualification. #>
[CmdletBinding(DefaultParameterSetName='Inputs')]
param(
    [Parameter(Mandatory,ParameterSetName='Inputs')][string]$InputRoot,
    [Parameter(ParameterSetName='Inputs')][ValidateSet('all','filament','luxcore')][string]$Backend='all',
    [Parameter(Mandatory,ParameterSetName='Fixture')][switch]$RunFixtureTests,
    [Parameter(Mandatory,ParameterSetName='Fixture')][string]$FixtureDirectory
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repoRoot=Split-Path -Parent $PSScriptRoot
if($RunFixtureTests) {
    if(-not (Test-Path -LiteralPath (Join-Path $FixtureDirectory 'tn_fixture.dll'))) {
        throw 'Build the CPU fixture explicitly with tools/build-renderers.ps1 -Fixture first.'
    }
    $priorFixture=$env:TN_RENDERER_FIXTURE_DIR
    try {
        $env:TN_RENDERER_FIXTURE_DIR=(Resolve-Path -LiteralPath $FixtureDirectory).Path
        Push-Location $repoRoot
        try {
            & cargo test --offline --test embedded_loader
            if($LASTEXITCODE -ne 0) { throw 'CPU fixture contract tests failed.' }
        } finally { Pop-Location }
    } finally { $env:TN_RENDERER_FIXTURE_DIR=$priorFixture }
    Write-Output 'CPU fixture contracts passed. This does not qualify a production renderer.'
    return
}
$inputBase=(Resolve-Path -LiteralPath $InputRoot).Path
$prefix=$inputBase.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
$lock=Get-Content -LiteralPath (Join-Path $repoRoot 'native/sdk-lock.json') -Raw | ConvertFrom-Json
function Assert-PinnedFile($record) {
    if([IO.Path]::IsPathRooted($record.path)) { throw "Unexpected absolute lock path: $($record.path)" }
    $path=[IO.Path]::GetFullPath((Join-Path $inputBase $record.path))
    if(-not $path.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase)) { throw 'Lock input escapes SDK root.' }
    if(-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing pinned input: $($record.path)" }
    if($record.PSObject.Properties.Name -contains 'size' -and (Get-Item -LiteralPath $path).Length -ne $record.size) {
        throw "Pinned input length mismatch: $($record.path)"
    }
    $digest=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if($digest -ne $record.sha256) { throw "Pinned input SHA-256 mismatch: $($record.path)" }
    return $path
}
$count=0
foreach($record in @($lock.archives)+@($lock.inputs)) {
    if($Backend -eq 'all' -or $record.backend -eq $Backend) {
        $null=Assert-PinnedFile $record
        ++$count
    }
}
foreach($manifest in $lock.header_manifests) {
    if($Backend -eq 'all' -or $manifest.backend -eq $Backend) {
        $manifestPath=Assert-PinnedFile $manifest
        $headers=Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if($headers.Count -eq 0 -or $headers.Count -gt 25000) { throw 'Invalid header inventory count.' }
        foreach($header in $headers) { $null=Assert-PinnedFile $header; ++$count }
    }
}
Write-Output "Verified $count pinned archive/runtime/tool/support-source/header inputs for $Backend."
Write-Output 'Input integrity only: production bridges, actual loaded module identities, GPU behavior and redistribution remain unqualified.'
