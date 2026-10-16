[CmdletBinding(DefaultParameterSetName='Production')]
param(
    [Parameter(Mandatory,ParameterSetName='Fixture')][switch]$Fixture,
    [Parameter(Mandatory,ParameterSetName='Production')][ValidateSet('filament','luxcore')][string]$Backend,
    [Parameter(Mandatory,ParameterSetName='Production')][string]$InputRoot,
    [string]$BuildDirectory
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repoRoot=Split-Path -Parent $PSScriptRoot
$lock=Get-Content -LiteralPath (Join-Path $repoRoot 'native/sdk-lock.json') -Raw | ConvertFrom-Json
if (-not $IsWindows) { throw 'Native SDK builds require Windows x64 MSVC.' }
if (-not $Fixture -and -not (Test-Path -LiteralPath (Join-Path $repoRoot "native/$Backend/CMakeLists.txt"))) {
    throw "Production $Backend bridge is not implemented. Fixture builds are explicitly test-only."
}
$cmake=(Get-Command cmake -ErrorAction Stop).Source
$version=& $cmake --version
if ($LASTEXITCODE -ne 0 -or $version[0] -ne "cmake version $($lock.toolchain.cmake)") {
    throw "Use pinned CMake $($lock.toolchain.cmake)."
}
if (-not $BuildDirectory) {
    $BuildDirectory=Join-Path $repoRoot $(if($Fixture){'artifacts/native-fixtures'}else{"artifacts/native-$Backend"})
}
$configure=@('-S',(Join-Path $repoRoot 'native'),'-B',$BuildDirectory,'-G',$lock.toolchain.generator,
    '-A','x64','-T',"v143,version=$($lock.toolchain.toolset)","-DCMAKE_SYSTEM_VERSION=$($lock.toolchain.windows_sdk)")
if($Fixture) {
    $configure+='-DTN_BUILD_FIXTURE=ON'
} else {
    & (Join-Path $PSScriptRoot 'qualify-renderers.ps1') -InputRoot $InputRoot -Backend $Backend
    $configure+=@('-DTN_BUILD_FIXTURE=OFF',"-DTN_BACKEND=$Backend","-DTN_SDK_ROOT=$((Resolve-Path -LiteralPath $InputRoot).Path)")
}
& $cmake @configure
if ($LASTEXITCODE -ne 0) { throw 'Native renderer CMake configuration failed.' }
& $cmake --build $BuildDirectory --config Release
if ($LASTEXITCODE -ne 0) { throw 'Native renderer build failed.' }
if($Fixture) {
    $fixturePath=Join-Path ([IO.Path]::GetFullPath($BuildDirectory)) 'fixture/Release'
    Write-Output "CPU fixture DLLs only: $fixturePath"
    Write-Output "Set TN_RENDERER_FIXTURE_DIR to this directory before running cargo test --offline --test embedded_loader."
} else {
    Write-Output 'Build completed; production runtime, visual, cancellation and packaging qualification is still required.'
}
