[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArchiveDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [switch]$AllowToolDownload
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$archiveRoot=[IO.Path]::GetFullPath($ArchiveDirectory)
$outputRoot=[IO.Path]::GetFullPath($OutputDirectory)
if(Test-Path -LiteralPath $outputRoot){throw 'Verified input directory must be fresh'}
function Assert-Hash([string]$Path,[string]$Expected){
    if((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Expected){throw "Hash mismatch: $Path"}
}
# This separately acquired header-only cache archive is mandatory even when
# downloading missing PyPI tools is permitted. Conan revision selection alone
# cannot replace the pinned payload SHA256 check.
$extra=Join-Path $archiveRoot 'getopt-exact.tgz'
if(!(Test-Path -LiteralPath $extra -PathType Leaf)){throw 'Missing SHA-verified getopt-exact.tgz; supply the qualified offline archive. AllowToolDownload applies only to locked PyPI tools.'}
$extraHash=(Get-Content (Join-Path $PSScriptRoot 'getopt-archive.sha256') -Raw).Trim()
Assert-Hash $extra $extraHash
$lock=Get-Content (Join-Path $PSScriptRoot 'tools-lock.json') -Raw | ConvertFrom-Json
foreach($item in $lock.artifacts){
    $source=Join-Path $archiveRoot $item.filename
    if(!(Test-Path -LiteralPath $source -PathType Leaf)){
        if(!$AllowToolDownload){throw "Missing pinned tool archive: $source"}
        Invoke-WebRequest -Uri $item.url -OutFile $source
    }
    Assert-Hash $source $item.sha256
}
New-Item -ItemType Directory -Path $outputRoot | Out-Null
$bootstrap=@(); $requirements=@()
foreach($item in $lock.artifacts){
    $destination=Join-Path $outputRoot $item.filename
    Copy-Item -LiteralPath (Join-Path $archiveRoot $item.filename) -Destination $destination
    Assert-Hash $destination $item.sha256
    # Explicit file requirements plus required hashes/no-deps prevent pip from
    # resolving any unlisted local wheel or transitive package candidate.
    $requirement=([Uri]$destination).AbsoluteUri+' --hash=sha256:'+$item.sha256
    if($item.name -in @('setuptools','wheel')){$bootstrap+=$requirement}
    $requirements+=$requirement
}
$verifiedExtra=Join-Path $outputRoot 'getopt-exact.tgz'
Copy-Item -LiteralPath $extra -Destination $verifiedExtra
Assert-Hash $verifiedExtra $extraHash
$bootstrap | Set-Content (Join-Path $outputRoot 'bootstrap-requirements.txt')
$requirements | Set-Content (Join-Path $outputRoot 'tools-requirements.txt')
