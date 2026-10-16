[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ArchiveDirectory,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$Materializer=(Join-Path $PSScriptRoot 'materialize-inputs.ps1')
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$archiveRoot=(Resolve-Path -LiteralPath $ArchiveDirectory).Path
$outputRoot=[IO.Path]::GetFullPath($OutputDirectory)
if(Test-Path -LiteralPath $outputRoot){throw 'Test output must be fresh'}
New-Item -ItemType Directory $outputRoot | Out-Null
function Fixture([string]$Name,[bool]$WithGetopt=$true){
    $path=Join-Path $outputRoot $Name
    New-Item -ItemType Directory $path | Out-Null
    foreach($item in (Get-Content (Join-Path $PSScriptRoot 'tools-lock.json') -Raw | ConvertFrom-Json).artifacts){
        Copy-Item -LiteralPath (Join-Path $archiveRoot $item.filename) -Destination $path
    }
    if($WithGetopt){Copy-Item -LiteralPath (Join-Path $archiveRoot 'getopt-exact.tgz') -Destination $path}
    return $path
}
function Rejected([string]$Source,[string]$Target,[string]$Expected){
    try{& $Materializer -ArchiveDirectory $Source -OutputDirectory $Target -AllowToolDownload}
    catch{if($_.Exception.Message -notmatch $Expected){throw};return}
    throw "INVALID_INPUT_ACCEPTED: $Source"
}
$poisoned=Fixture 'poisoned'
# A valid compatible newer wheel, inert unless a resolver wrongly selects it.
$wheel=[IO.Compression.ZipFile]::Open((Join-Path $poisoned 'requests-2.99.0-py3-none-any.whl'),'Create')
try{
    $entries=@{
        'requests/__init__.py'="__version__ = '2.99.0'`n"
        'requests-2.99.0.dist-info/METADATA'="Metadata-Version: 2.1`nName: requests`nVersion: 2.99.0`n"
        'requests-2.99.0.dist-info/WHEEL'="Wheel-Version: 1.0`nGenerator: provenance-test`nRoot-Is-Purelib: true`nTag: py3-none-any`n"
        'requests-2.99.0.dist-info/RECORD'=''
    }
    foreach($name in $entries.Keys){$writer=[IO.StreamWriter]::new($wheel.CreateEntry($name).Open());try{$writer.Write($entries[$name])}finally{$writer.Dispose()}}
}finally{$wheel.Dispose()}
$verified=Join-Path $outputRoot 'verified'
& $Materializer -ArchiveDirectory $poisoned -OutputDirectory $verified
if(Test-Path (Join-Path $verified 'requests-2.99.0-py3-none-any.whl')){throw 'UNLOCKED_WHEEL_MATERIALIZED'}
$requirements=Get-Content (Join-Path $verified 'tools-requirements.txt')
if($requirements.Count -ne 16 -or @($requirements | Where-Object {$_ -notmatch ' --hash=sha256:[a-f0-9]{64}$'}).Count){throw 'Incomplete hash-required tool set'}
$corrupt=Fixture 'corrupt-getopt'
$file=Join-Path $corrupt 'getopt-exact.tgz';$bytes=[IO.File]::ReadAllBytes($file);$bytes[0]=$bytes[0] -bxor 1;[IO.File]::WriteAllBytes($file,$bytes)
Rejected $corrupt (Join-Path $outputRoot 'reject-getopt') 'Hash mismatch:.*getopt-exact'
$missing=Fixture 'missing-getopt' $false
Rejected $missing (Join-Path $outputRoot 'reject-missing') 'Missing SHA-verified getopt'
$badWheel=Fixture 'corrupt-wheel'
$file=Join-Path $badWheel 'requests-2.34.2-py3-none-any.whl';$bytes=[IO.File]::ReadAllBytes($file);$bytes[0]=$bytes[0] -bxor 1;[IO.File]::WriteAllBytes($file,$bytes)
Rejected $badWheel (Join-Path $outputRoot 'reject-wheel') 'Hash mismatch:.*requests'
'BUILD_INPUTS_PASS unlocked wheel excluded; corrupt wheel/getopt rejected; missing getopt cannot download'
