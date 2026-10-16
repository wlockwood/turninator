[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$BuildRoot,
    [Parameter(Mandatory)][string]$QualificationRoot,
    [ValidateSet('prepare','cold','warm','edits','cancel','bounded64','edge-diag','parallel')][string]$Mode='prepare'
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repo=(Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$root=(Resolve-Path -LiteralPath $BuildRoot).Path
$q=(Resolve-Path -LiteralPath $QualificationRoot).Path
if(!$root.StartsWith((Join-Path $repo 'artifacts')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'BuildRoot must be an owned artifact directory'}
$runtime=Join-Path $root 'runtime'
function Run([string]$Program,[string[]]$Arguments,[string]$Log,[int]$Expected=0){
    & $Program @Arguments *> $Log
    if($LASTEXITCODE -ne $Expected){Get-Content $Log -Tail 25;throw "$Program returned $LASTEXITCODE (expected $Expected)"}
}
if($Mode -eq 'prepare'){
    $dll=Get-ChildItem (Join-Path $root 'build') -Filter luxcore.dll -Recurse | Select-Object -First 1
    $lib=Get-ChildItem (Join-Path $root 'build') -Filter luxcore.lib -Recurse | Where-Object DirectoryName -EQ $dll.DirectoryName | Select-Object -First 1
    if(!$dll -or !$lib){throw 'Corrected SDK output missing'}
    $boost=Get-ChildItem (Join-Path $root 'conan/p') -Filter version.hpp -Recurse | Where-Object FullName -Match '[/\\]include[/\\]boost[/\\]version.hpp$' | Select-Object -First 1
    if(!$boost){throw 'Exact Boost headers missing'}
    $tests=Join-Path $root 'qualification-source'
    New-Item -ItemType Directory -Force $tests,$runtime | Out-Null
    Copy-Item (Join-Path $PSScriptRoot '*cpp') $tests
    Copy-Item (Join-Path $PSScriptRoot 'owned_ptx_cache.h') $tests
    Copy-Item (Join-Path $PSScriptRoot 'tests-CMakeLists.txt') (Join-Path $tests 'CMakeLists.txt')
    $checks=Join-Path $root 'qualification-build'
    $boostInclude=Split-Path (Split-Path $boost.FullName)
    Run 'cmake' @('-S',$tests,'-B',$checks,'-G','Visual Studio 17 2022','-A','x64',"-DSDK_SOURCE=$root/source","-DSDK_GENERATED=$root/build/generated/include","-DBOOST_INCLUDE=$boostInclude","-DSDK_LIBRARY=$($lib.FullName)") (Join-Path $root 'consumer-configure.log')
    Run 'cmake' @('--build',$checks,'--config','Release','--parallel','2') (Join-Path $root 'consumer-build.log')
    $lock=Get-Content (Join-Path $repo 'native/sdk-lock.json') -Raw | ConvertFrom-Json
    foreach($moduleInput in $lock.inputs | Where-Object { $_.backend -eq 'luxcore' -and $_.path -like 'luxcore/native/LuxCore/bin/*.dll' -and $_.path -notlike '*luxcore.dll' -and $_.path -notlike '*.alt.dll' }){
        $file=Join-Path $q $moduleInput.path
        if((Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $moduleInput.sha256){throw "Runtime module identity differs: $file"}
        Copy-Item -LiteralPath $file -Destination $runtime
    }
    Copy-Item -LiteralPath $dll.FullName -Destination $runtime
    Copy-Item (Join-Path $checks 'Release/*.exe') $runtime
    $sdk=(Get-FileHash (Join-Path $runtime 'luxcore.dll') -Algorithm SHA256).Hash.ToLowerInvariant()
    $nvrtc=(Get-FileHash (Join-Path $runtime 'nvrtc64_120_0.dll') -Algorithm SHA256).Hash.ToLowerInvariant()
    $builtins=(Get-FileHash (Join-Path $runtime 'nvrtc-builtins64_128.dll') -Algorithm SHA256).Hash.ToLowerInvariant()
    $compiler=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes("$nvrtc`n$builtins`n"))).ToLowerInvariant()
    @{sdk_identity=$sdk;compiler_identity=$compiler;nvrtc_sha256=$nvrtc;builtins_sha256=$builtins;cache_directory=(Join-Path $root 'cache-é-層')} | ConvertTo-Json | Set-Content (Join-Path $root 'runtime-identities.json')
    Push-Location $runtime
    try { Run './direct_consumer.exe' @() (Join-Path $root 'direct-consumer.log') } finally {Pop-Location}
    Write-Host 'Direct native consumer linked and passed; GPU cases require coordinator slot.'
    exit 0
}
$identities=Get-Content (Join-Path $root 'runtime-identities.json') -Raw | ConvertFrom-Json
$namespace=Join-Path $identities.cache_directory "turninator-luxcore-ptx-v1/$($identities.compiler_identity)/$($identities.sdk_identity)"
if($Mode -eq 'cold' -and (Test-Path -LiteralPath $namespace)){throw 'Cold qualification requires an absent owned identity namespace'}
Push-Location $runtime
try {
    $expected=if($Mode -eq 'edge-diag'){21}else{0}
    Run './render_probe.exe' @("--$Mode",$identities.cache_directory,$identities.compiler_identity,$identities.sdk_identity) (Join-Path $root "render-$Mode.log") $expected
} finally {Pop-Location}
Run 'python' @((Join-Path $PSScriptRoot 'inspect-cache.py'),$root) (Join-Path $root "cache-$Mode.json")
