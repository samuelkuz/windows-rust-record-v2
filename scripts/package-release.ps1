param(
    [string]$FfmpegBinDir,
    [string]$DistDir = "dist"
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$manifestPath = Join-Path $repoRoot "Cargo.toml"
$manifest = Get-Content -LiteralPath $manifestPath -Raw
$versionMatch = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    throw "Could not find package version in Cargo.toml"
}

$version = $versionMatch.Groups[1].Value
$appName = "WindowsRustRecord"
$binaryName = "windows-rust-record-v2.exe"
$packageName = "$appName-$version-windows-x64"
$distRoot = Join-Path $repoRoot $DistDir
$packageRoot = Join-Path $distRoot $packageName
$zipPath = Join-Path $distRoot "$packageName.zip"

if (-not $FfmpegBinDir) {
    $pathFfmpeg = Get-Command "ffmpeg.exe" -ErrorAction SilentlyContinue
    $pathFfprobe = Get-Command "ffprobe.exe" -ErrorAction SilentlyContinue
    if ($pathFfmpeg -and $pathFfprobe) {
        $FfmpegBinDir = Split-Path -Parent $pathFfmpeg.Source
    } else {
        $candidateDirs = @(
            (Join-Path $repoRoot "vendor\ffmpeg\bin"),
            (Join-Path $repoRoot "ffmpeg\bin")
        )

        foreach ($candidateDir in $candidateDirs) {
            if ((Test-Path -LiteralPath (Join-Path $candidateDir "ffmpeg.exe")) -and
                (Test-Path -LiteralPath (Join-Path $candidateDir "ffprobe.exe"))) {
                $FfmpegBinDir = $candidateDir
                break
            }
        }
    }
}

if (-not $FfmpegBinDir) {
    throw "Provide -FfmpegBinDir pointing at a folder with ffmpeg.exe and ffprobe.exe, or put them on PATH."
}

$FfmpegBinDir = (Resolve-Path -LiteralPath $FfmpegBinDir).Path
$ffmpegExe = Join-Path $FfmpegBinDir "ffmpeg.exe"
$ffprobeExe = Join-Path $FfmpegBinDir "ffprobe.exe"
if (-not (Test-Path -LiteralPath $ffmpegExe)) {
    throw "Missing ffmpeg.exe in $FfmpegBinDir"
}
if (-not (Test-Path -LiteralPath $ffprobeExe)) {
    throw "Missing ffprobe.exe in $FfmpegBinDir"
}

Push-Location $repoRoot
try {
    cargo build --release
} finally {
    Pop-Location
}

$releaseExe = Join-Path $repoRoot "target\release\$binaryName"
if (-not (Test-Path -LiteralPath $releaseExe)) {
    throw "Release executable was not produced at $releaseExe"
}

if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}

New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot "ffmpeg\bin") | Out-Null

Copy-Item -LiteralPath $releaseExe -Destination (Join-Path $packageRoot $binaryName)
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $packageRoot "README.md")
Copy-Item -LiteralPath $ffmpegExe -Destination (Join-Path $packageRoot "ffmpeg\bin\ffmpeg.exe")
Copy-Item -LiteralPath $ffprobeExe -Destination (Join-Path $packageRoot "ffmpeg\bin\ffprobe.exe")

$optionalFiles = @("LICENSE", "LICENSE.txt", "NOTICE", "NOTICE.txt")
foreach ($optionalFile in $optionalFiles) {
    $source = Join-Path $repoRoot $optionalFile
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination (Join-Path $packageRoot $optionalFile)
    }
}

$ffmpegLicenseFiles = @("LICENSE", "LICENSE.txt", "COPYING.GPLv2", "COPYING.GPLv3", "COPYING.LGPLv2.1", "COPYING.LGPLv3")
foreach ($licenseFile in $ffmpegLicenseFiles) {
    $source = Join-Path $FfmpegBinDir "..\$licenseFile"
    if (Test-Path -LiteralPath $source) {
        Copy-Item -LiteralPath $source -Destination (Join-Path $packageRoot "ffmpeg\$licenseFile")
    }
}

Compress-Archive -LiteralPath $packageRoot -DestinationPath $zipPath -Force

Write-Host "Packaged $packageName"
Write-Host "Folder: $packageRoot"
Write-Host "Zip:    $zipPath"
