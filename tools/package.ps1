[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')][string]$Profile = 'release',
    [switch]$SkipBuild,
    [string]$ExecutablePath = '',
    [string]$PackageName = '',
    [ValidateSet('full', 'core')][string]$Variant = 'full',
    [string]$NativeSdkManifest = '',
    [string]$NativeRuntimeDirectory = '',
    [string]$Python = 'python',
    [string]$Blender = 'C:/Program Files/Blender Foundation/Blender 4.5/blender.exe',
    [string]$FFmpeg = 'C:/Program Files/ffmpeg/bin/ffmpeg.exe',
    [string]$FFprobe = 'C:/Program Files/ffmpeg/bin/ffprobe.exe'
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($ExecutablePath -and -not $SkipBuild) {
    throw 'ExecutablePath requires -SkipBuild; it packages an explicitly selected prebuilt executable.'
}
if ($PackageName -and $PackageName -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,100}$') {
    throw 'PackageName must be a single safe directory name.'
}
if ($Variant -eq 'full' -and (-not $NativeSdkManifest -or -not $NativeRuntimeDirectory)) {
    throw 'Full packaging requires -NativeSdkManifest and -NativeRuntimeDirectory from a reviewed native build.'
}
if ($Variant -eq 'core' -and ($NativeSdkManifest -or $NativeRuntimeDirectory)) {
    throw 'Core packaging must not accept optional native SDK inputs.'
}
$nativeArgs = @((Join-Path $PSScriptRoot 'native_package.py'), '--variant', $Variant)
if ($Variant -eq 'full') {
    $nativeArgs += @('--manifest', [IO.Path]::GetFullPath($NativeSdkManifest), '--runtime', [IO.Path]::GetFullPath($NativeRuntimeDirectory))
}
# Validate the closed native inventory before Cargo, build work or package writes.
& $Python @nativeArgs
if ($LASTEXITCODE -ne 0) { throw 'Native SDK inventory validation failed.' }
function Assert-WorkspacePath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($workspace + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Package path escapes workspace: $full"
    }
    $itemPath = $full
    while ($itemPath.Length -ge $workspace.Length) {
        if (Test-Path -LiteralPath $itemPath) {
            $item = Get-Item -LiteralPath $itemPath -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Package paths must not contain reparse points: $itemPath"
            }
        }
        $itemPath = Split-Path -Parent $itemPath
    }
    return $full
}
Push-Location -LiteralPath $workspace
try {
    $target = 'x86_64-pc-windows-msvc'
    $hostInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or -not ($hostInfo -contains "host: $target")) {
        throw "Packaging requires the native $target Rust toolchain."
    }
    $metadataRaw = & cargo metadata --offline --locked --format-version 1 --filter-platform $target
    if ($LASTEXITCODE -ne 0) { throw 'Locked Cargo metadata failed.' }
    $metadata = $metadataRaw | ConvertFrom-Json
    $application = $metadata.packages | Where-Object id -EQ $metadata.resolve.root
    if (-not $PackageName) {
        $PackageName = 'turninator-' + $application.version + '-windows-x64-' + $Variant + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    }
    $releaseRoot = Assert-WorkspacePath (Join-Path $workspace 'artifacts/releases')
    $package = Assert-WorkspacePath (Join-Path $releaseRoot $PackageName)
    $archive = Assert-WorkspacePath ($package + '.zip')
    if ((Test-Path -LiteralPath $package) -or (Test-Path -LiteralPath $archive)) {
        throw 'Package destination already exists; choose a new name.'
    }
    if (-not $SkipBuild) {
        $buildArgs = @('build', '--offline', '--locked', '--bin', 'turninator')
        if ($Profile -eq 'release') { $buildArgs += '--release' }
        & cargo @buildArgs
        if ($LASTEXITCODE -ne 0) { throw 'Application build failed.' }
    }
    $executable = if ($ExecutablePath) { [IO.Path]::GetFullPath($ExecutablePath) } else { Join-Path $metadata.target_directory "$Profile/turninator.exe" }
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Missing executable: $executable" }
    $resources = @('renderer/worker.py', 'renderer/scene.py', 'renderer/frame_pipeline.py', 'docs/runtime-support.md', 'docs/renderers/qualification-matrix.md', 'docs/renderers/filament.md', 'docs/renderers/luxcore.md', 'docs/renderers/luxcore-native.md', 'docs/performance.md')
    foreach ($resource in ($resources + @('README.md'))) {
        if (-not (Test-Path -LiteralPath (Join-Path $workspace $resource) -PathType Leaf)) { throw "Missing runtime resource: $resource" }
    }
    New-Item -ItemType Directory -Path $package | Out-Null
    & $Python @nativeArgs --output $package
    if ($LASTEXITCODE -ne 0) { throw 'Native SDK staging failed; incomplete directory is not a release.' }
    New-Item -ItemType Directory -Path (Join-Path $package 'renderer'), (Join-Path $package 'docs'), (Join-Path $package 'docs/renderers') | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $package 'turninator.exe')
    foreach ($resource in $resources) {
        Copy-Item -LiteralPath (Join-Path $workspace $resource) -Destination (Join-Path $package $resource)
    }
    Copy-Item -LiteralPath (Join-Path $workspace 'README.md') -Destination (Join-Path $package 'docs/source-readme.md')
    $metadataPath = Join-Path $package 'cargo-metadata.json'
    # Metadata is temporary: published inventory must not reveal local Cargo-cache paths.
    [IO.File]::WriteAllText($metadataPath, ($metadataRaw -join "`n"), [Text.UTF8Encoding]::new($false))
    $noticesPath = Join-Path $package 'THIRD-PARTY-NOTICES.md'
    & $Python (Join-Path $PSScriptRoot 'generate-notices.py') --metadata $metadataPath --output $noticesPath --native-manifest (Join-Path $package 'native-sdk-manifest.json') --native-root $package
    if ($LASTEXITCODE -ne 0) { throw 'Dependency/font notice generation failed.' }
    $readme = @'
# Turninator portable Windows build

Extract the entire folder, then double-click turninator.exe. Keep the renderer
folder beside it. The application can be launched from another working directory.
Native STL/G-code inspection works without Blender, FFmpeg, Python or Rust installed.

Native wgpu rendering is embedded. Full packages also carry Filament and LuxCore
under renderers; core development packages omit these optional SDKs and report
them unavailable. native-sdk-manifest.json identifies the actual package variant
and native payload. Native SDK presence does not promise support on every GPU.
Blender EEVEE/Cycles use a separate Blender installation. GIF/MP4 encoding needs
separately installed FFmpeg/ffprobe. Configure full executable paths in Tools /
storage. Missing tools are reported and are not downloaded. See the runtime
documentation for device qualification, approximations and output requirements.
For 4096-pixel LuxCore output with denoising, increase the host memory maximum
from 8 GiB to 16 GiB in Tools / storage before rendering. This is an allocation
ceiling, not a claim about installed or free memory.

By default settings and retained jobs live in %LOCALAPPDATA%/Turninator. A fresh,
self-contained data location can be selected from PowerShell:

    .\turninator.exe gui --data-dir "D:\My Turninator Data"

Open a model, inspect it, and use Render preview or the GIF/PNG/4-view/MP4 buttons.
Use Tools / storage to manage retained jobs. Imported source files remain unchanged.
docs/source-readme.md contains the source checkout's usage and limitations.

version-manifest.json records package file hashes, source hashes, the resolved
dependency versions, compiler identity, and external binaries observed on the build
machine. Those tools are not bundled. THIRD-PARTY-NOTICES.md contains the dependency
and embedded-font notices, plus native component notices and source availability.
This package does not assign a new license to Turninator
or to imported models. The manifest excludes itself and the ZIP archive.
'@
    [IO.File]::WriteAllText((Join-Path $package 'README.md'), $readme, [Text.UTF8Encoding]::new($false))
    # Keep metadata outside the package while generating the final inventory.
    $temporaryMetadata = Assert-WorkspacePath (Join-Path $releaseRoot ($PackageName + '.metadata.json'))
    if (Test-Path -LiteralPath $temporaryMetadata) { throw 'Metadata scratch path already exists.' }
    Move-Item -LiteralPath $metadataPath -Destination $temporaryMetadata
    $manifestArgs = @((Join-Path $PSScriptRoot 'generate-notices.py'), '--metadata', $temporaryMetadata,
        '--output', (Join-Path $package 'THIRD-PARTY-NOTICES.md'), '--package', $package,
        '--source-root', $workspace, '--profile', $Profile, '--tool', $Blender, '--tool', $FFmpeg, '--tool', $FFprobe)
    $manifestArgs += @('--native-manifest', (Join-Path $package 'native-sdk-manifest.json'), '--native-root', $package)
    if ($SkipBuild) { $manifestArgs += '--prebuilt' }
    & $Python @manifestArgs
    if ($LASTEXITCODE -ne 0) { throw 'Version manifest generation failed.' }
    Remove-Item -LiteralPath $temporaryMetadata
    Compress-Archive -LiteralPath $package -DestinationPath $archive -CompressionLevel Optimal
    Write-Output "Portable package: $package"
    Write-Output "Archive: $archive"
    Write-Output "Archive SHA256: $((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash)"
} finally {
    Pop-Location
}
