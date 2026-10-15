[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$QualificationRoot,
    [string]$OutputRoot = 'artifacts/luxcore-sdk-rebuild',
    [string]$ToolArchives = 'artifacts/luxcore-sdk/tool-wheels',
    [switch]$AllowToolDownload,
    [ValidateRange(1,16)][int]$Parallel = 4,
    [switch]$ConfigureOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$support = Join-Path $repo 'native/patches/luxcore'
$output = [IO.Path]::GetFullPath((Join-Path $repo $OutputRoot))
$allowed = [IO.Path]::GetFullPath((Join-Path $repo 'artifacts')) + [IO.Path]::DirectorySeparatorChar
if (!$output.StartsWith($allowed,[StringComparison]::OrdinalIgnoreCase)) { throw 'OutputRoot must be below this worktree artifacts directory' }
if (Test-Path -LiteralPath $output) { throw 'OutputRoot must be fresh; existing outputs are never deleted or reused by this recipe' }
$q = (Resolve-Path -LiteralPath $QualificationRoot).Path
$archive = Join-Path $q 'luxcore-source.tar.gz'
$depsZip = Join-Path $q 'luxcore-deps-windows-2022.zip'
function Assert-Hash([string]$Path,[string]$Expected) {
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected) { throw "Hash mismatch: $Path" }
}
function Invoke-Checked([string]$Program,[string[]]$Arguments,[string]$Log) {
    & $Program @Arguments *> $Log
    if ($LASTEXITCODE) { Get-Content -LiteralPath $Log -Tail 30; throw "$Program exited $LASTEXITCODE; see $Log" }
}
$hostTools = Get-Content (Join-Path $support 'host-tools.json') -Raw | ConvertFrom-Json
foreach ($name in @('cmake.exe','python.exe')) {
    $expected = $hostTools | Where-Object name -EQ $name
    Assert-Hash (Get-Command $name).Source $expected.sha256
}
Assert-Hash $archive '31a82d87c1f912864e7b039620747507d2b0ce9db70648656d41d0ebc4636cda'
Assert-Hash $depsZip '069a1eace7e8f192b07629d6318eca9e0ad970f8c7db39a0919b10eddc2345d0'
New-Item -ItemType Directory -Path $output | Out-Null
$toolsDir = [IO.Path]::GetFullPath((Join-Path $repo $ToolArchives))
if (!$toolsDir.StartsWith($allowed,[StringComparison]::OrdinalIgnoreCase)) { throw 'ToolArchives must be in this worktree artifacts directory' }
New-Item -ItemType Directory -Force -Path $toolsDir | Out-Null
$toolsLock = Get-Content (Join-Path $support 'tools-lock.json') -Raw | ConvertFrom-Json
foreach ($item in $toolsLock.artifacts) {
    $local = Join-Path $toolsDir $item.filename
    if (!(Test-Path -LiteralPath $local)) {
        if (!$AllowToolDownload) { throw "Missing pinned tool archive $local; supply it or use -AllowToolDownload" }
        Invoke-WebRequest -Uri $item.url -OutFile $local
    }
    Assert-Hash $local $item.sha256
}
$venv = Join-Path $output 'venv'
Invoke-Checked 'python' @('-m','venv',$venv) (Join-Path $output 'venv.log')
$python = Join-Path $venv 'Scripts/python.exe'
$conan = Join-Path $venv 'Scripts/conan.exe'
$pipArgs = @('-m','pip','install','--cache-dir',(Join-Path $output 'pip-cache'),'--no-index','--no-build-isolation','--find-links',$toolsDir)
Invoke-Checked $python ($pipArgs + @('setuptools==80.9.0','wheel==0.45.1')) (Join-Path $output 'bootstrap.log')
Invoke-Checked $python ($pipArgs + @('conan==2.18.1')) (Join-Path $output 'tool-install.log')
$source = Join-Path $output 'source'
Invoke-Checked $python @((Join-Path $support 'extract-source.py'),$archive,$source) (Join-Path $output 'extract.log')
# Extract the original ZIP into our own tree: the existing unpacked cache is never mutated.
$deps = Join-Path $output 'deps'
Expand-Archive -LiteralPath $depsZip -DestinationPath $deps
$cacheArchive = Get-ChildItem -LiteralPath $deps -Filter conan-cache-save.tgz -Recurse | Select-Object -First 1
if (!$cacheArchive) { throw 'Pinned dependency archive has no Conan cache' }
$savedConanHome = $env:CONAN_HOME
try {
    $env:CONAN_HOME = Join-Path $output 'conan'
    Invoke-Checked $conan @('cache','restore',$cacheArchive.FullName) (Join-Path $output 'restore.log')
    # Header-only package omitted from the official archive, referenced exactly
    # by giflib's conaninfo.txt. No compiled dependency can be rebuilt here.
    $getopt = 'getopt-for-visual-studio/20200201#32ee360e2552fe65ad8bec52edcd64fd:da39a3ee5e6b4b0d3255bfef95601890afd80709#26eb4a9cdd6e4c7b9d2c892b970f14d3'
    $extra = Join-Path $toolsDir 'getopt-exact.tgz'
    if (Test-Path -LiteralPath $extra) {
        Assert-Hash $extra (Get-Content (Join-Path $support 'getopt-archive.sha256') -Raw).Trim()
        Invoke-Checked $conan @('cache','restore',$extra) (Join-Path $output 'getopt-restore.log')
    } elseif ($AllowToolDownload) {
        Invoke-Checked $conan @('download',$getopt,'-r','conancenter') (Join-Path $output 'getopt-download.log')
    } else { throw 'Missing pinned getopt-exact.tgz; supply the qualified archive or use -AllowToolDownload' }
    $profiles = Get-ChildItem -LiteralPath $env:CONAN_HOME -Filter conan-profile-common -Recurse | Select-Object -First 1
    if (!$profiles) { throw 'Restored dependency profile missing' }
    (Get-Content $profiles.FullName -Raw).Replace('tools.system.package_manager:mode=install','tools.system.package_manager:mode=check') | Set-Content (Join-Path $output 'conan-profile-common')
    Copy-Item -LiteralPath (Join-Path $support 'conan-profile') -Destination (Join-Path $output 'profile')
    Push-Location $source
    try {
        Invoke-Checked 'git' @('apply','--check','--',(Join-Path $support '0001-owned-cache-exports-native.patch')) (Join-Path $output 'patch-check.log')
        Invoke-Checked 'git' @('apply','--',(Join-Path $support '0001-owned-cache-exports-native.patch')) (Join-Path $output 'patch-apply.log')
        Invoke-Checked $python @((Join-Path $support 'normalize-patched-source.py'),$source,(Join-Path $support '0001-owned-cache-exports-native.patch')) (Join-Path $output 'normalize.log')
        Copy-Item -LiteralPath (Join-Path $support 'owned_ptx_cache.h') -Destination 'include/luxrays/utils/owned_ptx_cache.h'
        $profile = Join-Path $output 'profile'
        Invoke-Checked $conan @('install','build-system/conan','-pr:h',$profile,'-pr:b',$profile,'--build=never','--no-remote','-c','tools.graph:skip_build=True') (Join-Path $output 'resolve.log')
        $build = Join-Path $output 'build'
        Invoke-Checked 'cmake' @('--preset','conan-default','-B',$build,'-DTURNINATOR_NATIVE_ONLY=ON','-DLUXCORE_DEMOS=OFF','-DLUXCORE_TESTS=OFF',"-DCMAKE_CXX_FLAGS=/D_ITERATOR_DEBUG_LEVEL=0 /MP$Parallel") (Join-Path $output 'configure.log')
        $compilerFile = Get-ChildItem -LiteralPath (Join-Path $build 'CMakeFiles') -Filter CMakeCXXCompiler.cmake -Recurse | Select-Object -First 1
        $compilerText = Get-Content $compilerFile.FullName -Raw
        $compilerMatch = [regex]::Match($compilerText,'set\(CMAKE_CXX_COMPILER "([^"]+)"\)')
        if (!$compilerMatch.Success) { throw 'Could not identify configured compiler' }
        $compilerPath = $compilerMatch.Groups[1].Value
        Assert-Hash $compilerPath ($hostTools | Where-Object name -EQ 'cl.exe').sha256
        Assert-Hash (Join-Path (Split-Path $compilerPath) 'link.exe') ($hostTools | Where-Object name -EQ 'link.exe').sha256
        if (!$ConfigureOnly) {
            Invoke-Checked 'cmake' @('--build',$build,'--config','Release','--target','luxcore','--parallel','1') (Join-Path $output 'build.log')
            $outputs = Get-ChildItem -LiteralPath $build -Recurse -File | Where-Object { $_.Name -in @('luxcore.dll','luxcore.lib') } | ForEach-Object {
                @{path=[IO.Path]::GetRelativePath($output,$_.FullName);size=$_.Length;sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
            }
            @{outputs=@($outputs);qualified=$false;note='Build only; run direct consumer and GPU qualification before promotion.'} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $output 'build-output.json')
        }
    } finally { Pop-Location }
} finally { $env:CONAN_HOME = $savedConanHome }
Write-Host "Native SDK build recipe finished: $output"
