[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$OpenMpRuntime,
  [string]$SdkRoot='D:/code/turninator/.worktrees/luxcore-sdk/artifacts/luxcore-sdk',
  [string]$OutputRoot='artifacts/luxcore-native'
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repo=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$sdk=(Resolve-Path -LiteralPath $SdkRoot).Path
$output=[IO.Path]::GetFullPath((Join-Path $repo $OutputRoot))
if(!$output.StartsWith((Join-Path $repo 'artifacts')+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Output must be within this worktree artifacts directory'}
$lock=Get-Content (Join-Path $repo 'native/sdk-lock.json') -Raw | ConvertFrom-Json
$qualified=Get-Content (Join-Path $repo 'native/patches/luxcore/qualification.json') -Raw | ConvertFrom-Json
$runtime=Join-Path $output 'runtime'
New-Item -ItemType Directory -Force $output,$runtime,(Join-Path $output 'cache'),(Join-Path $output 'images') | Out-Null
$records=@()
$openmp=(Resolve-Path -LiteralPath $OpenMpRuntime).Path
$openmpHash=(Get-FileHash -LiteralPath $openmp -Algorithm SHA256).Hash.ToLowerInvariant()
if($openmpHash -ne '55aba23cdcd6484fbb06f4155b8ca75adfce7a881f10afd0c49457165e677164'){throw 'Qualified x64 Visual C++ OpenMP redistributable required'}
Copy-Item -LiteralPath $openmp -Destination (Join-Path $runtime 'vcomp140.dll')
$records+=@{path=$openmp;sha256=$openmpHash;staged_path=(Join-Path $runtime 'vcomp140.dll')}
foreach($entry in $lock.inputs | Where-Object { $_.backend -eq 'luxcore' -and $_.path -like 'luxcore/native/LuxCore/bin/*.dll' -and $_.path -notlike '*.alt.dll' }) {
  $name=Split-Path -Leaf $entry.path
  $input=Join-Path $sdk "runtime/$name"
  $expected=if($name -eq 'luxcore.dll'){$qualified.sdk_identity}else{$entry.sha256}
  $actual=(Get-FileHash -LiteralPath $input -Algorithm SHA256).Hash.ToLowerInvariant()
  if($actual -ne $expected){throw "Qualified runtime hash mismatch: $name"}
  Copy-Item -LiteralPath $input -Destination $runtime
  $records+=@{path=$input;sha256=$actual;staged_path=(Join-Path $runtime $name)}
}
$boost=Join-Path $sdk 'conan/p/boostca70ba476159c/p/include'
$oidn=Join-Path $sdk 'conan/p/oidn5fe95d8faf9d6/p'
& cmake -S (Join-Path $repo 'native') -B (Join-Path $output 'build') -G 'Visual Studio 17 2022' -A x64 -DTN_BACKEND=luxcore -DTN_LUXCORE_TESTS=ON "-DTN_SDK_ROOT=$sdk" "-DTN_LUXCORE_BOOST=$boost" "-DTN_LUXCORE_OIDN=$oidn" *> (Join-Path $output 'configure.log')
if($LASTEXITCODE){throw 'Native configuration failed'}
& cmake --build (Join-Path $output 'build') --config Release --parallel 2 *> (Join-Path $output 'build.log')
if($LASTEXITCODE){throw 'Native build failed'}
Copy-Item -LiteralPath (Join-Path $output 'build/luxcore/Release/tn_renderer_luxcore.dll'),(Join-Path $output 'build/luxcore/Release/luxcore_conformance.exe') -Destination $runtime
Copy-Item -LiteralPath (Join-Path $output 'build/luxcore/Release/luxcore_direct.exe') -Destination $runtime
foreach($input in @((Join-Path $oidn 'include/OpenImageDenoise/oidn.hpp'),(Join-Path $oidn 'include/OpenImageDenoise/oidn.h'),(Join-Path $oidn 'include/OpenImageDenoise/config.h'),(Join-Path $oidn 'lib/LuxOpenImageDenoise.lib'))) {
  $records+=@{path=$input;sha256=(Get-FileHash -LiteralPath $input -Algorithm SHA256).Hash.ToLowerInvariant()}
}
$records | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $output 'build-inputs.json')
Write-Output "Production native build staged at $runtime; GPU execution requires coordinator grant."
