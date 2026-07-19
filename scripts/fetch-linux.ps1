param(
    [string]$Destination = "$PSScriptRoot\..\third_party\linux-6.18.39",
    [switch]$KeepArchive
)

$ErrorActionPreference = "Stop"
$version = "6.18.39"
$url = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-$version.tar.xz"
$expectedSha256 = "a7a7e3d2ae9d95e74197223a8d4eb5f6be7aac21b6e6de27e9685d001c1f8cb0"
$destinationPath = [System.IO.Path]::GetFullPath($Destination)
$parent = Split-Path -Parent $destinationPath
$archive = Join-Path $parent "linux-$version.tar.xz"

if (Test-Path (Join-Path $destinationPath "Makefile")) {
    Write-Host "Linux $version already exists at $destinationPath"
    exit 0
}

New-Item -ItemType Directory -Force -Path $parent | Out-Null
if (-not (Test-Path $archive)) {
    Write-Host "[linux] downloading $url"
    curl.exe -L --fail --retry 3 --output $archive $url
}

$actualSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
    throw "Linux source checksum mismatch: expected $expectedSha256, got $actualSha256"
}

$temporary = "$destinationPath.tmp"
if (Test-Path $temporary) {
    Remove-Item -LiteralPath $temporary -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $temporary | Out-Null
tar.exe -xJf $archive -C $temporary
Move-Item -LiteralPath (Join-Path $temporary "linux-$version") -Destination $destinationPath
Remove-Item -LiteralPath $temporary -Recurse -Force

if (-not $KeepArchive) {
    Remove-Item -LiteralPath $archive -Force
}

Write-Host "[linux] extracted Linux $version to $destinationPath"
