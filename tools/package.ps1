[CmdletBinding()]
param(
    [ValidateSet('release', 'debug')][string]$Profile = 'release',
    [switch]$SkipBuild,
    [string]$ExecutablePath = '',
    [string]$PackageName = '',
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
        $PackageName = 'turninator-' + $application.version + '-windows-x64-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
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
    $executable = if ($ExecutablePath) { [IO.Path]::GetFullPath($ExecutablePath) } else { Join-Path $workspace "target/$Profile/turninator.exe" }
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Missing executable: $executable" }
    foreach ($resource in @('renderer/worker.py', 'renderer/scene.py', 'renderer/frame_pipeline.py', 'README.md', 'docs/runtime-support.md')) {
        if (-not (Test-Path -LiteralPath (Join-Path $workspace $resource) -PathType Leaf)) { throw "Missing runtime resource: $resource" }
    }
    New-Item -ItemType Directory -Path $package | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $package 'renderer'), (Join-Path $package 'docs') | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $package 'turninator.exe')
    foreach ($resource in @('renderer/worker.py', 'renderer/scene.py', 'renderer/frame_pipeline.py', 'docs/runtime-support.md')) {
        Copy-Item -LiteralPath (Join-Path $workspace $resource) -Destination (Join-Path $package $resource)
    }
    Copy-Item -LiteralPath (Join-Path $workspace 'README.md') -Destination (Join-Path $package 'docs/source-readme.md')
    $metadataPath = Join-Path $package 'cargo-metadata.json'
    # Metadata is temporary: published inventory must not reveal local Cargo-cache paths.
    [IO.File]::WriteAllText($metadataPath, ($metadataRaw -join "`n"), [Text.UTF8Encoding]::new($false))
    $noticesPath = Join-Path $workspace 'THIRD-PARTY-NOTICES.md'
    & $Python (Join-Path $PSScriptRoot 'generate-notices.py') --metadata $metadataPath --output $noticesPath
    if ($LASTEXITCODE -ne 0) { throw 'Dependency/font notice generation failed.' }
    Copy-Item -LiteralPath $noticesPath -Destination (Join-Path $package 'THIRD-PARTY-NOTICES.md')
    $readme = @'
# Turninator portable Windows build

Extract the entire folder, then double-click turninator.exe. Keep the renderer
folder beside it. The application can be launched from another working directory.
Native STL/G-code inspection works without Blender, FFmpeg, Python or Rust installed.

Rendering and export use separately installed Blender, FFmpeg and ffprobe. Open
Tools / storage in the app, enter each executable's full path, and Save settings.
See docs/runtime-support.md for exact tested versions, hashes and GPU requirements.
Cycles currently requires a supported NVIDIA OptiX GPU. EEVEE is the other explicit
render choice. Missing tools produce an actionable error; they are not downloaded.
No external executable is included in this package.

By default settings and retained jobs live in %LOCALAPPDATA%/Turninator. A fresh,
self-contained data location can be selected from PowerShell:

    .\turninator.exe gui --data-dir "D:\My Turninator Data"

Open a model, inspect it, and use Render preview or the GIF/PNG/4-view/MP4 buttons.
Use Tools / storage to manage retained jobs. Imported source files remain unchanged.
docs/source-readme.md contains the source checkout's usage and limitations.

version-manifest.json records package file hashes, source hashes, the resolved
dependency versions, compiler identity, and external binaries observed on the build
machine. Those tools are not bundled. THIRD-PARTY-NOTICES.md contains the dependency
and embedded-font notices. This package does not assign a new license to Turninator
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
