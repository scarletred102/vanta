[CmdletBinding()]
param(
    [switch]$Gpt = $true,
    [switch]$Legacy,
    [switch]$Headless,
    [int]$TimeoutSeconds = 0
)

# Build the vanta kernel and boot in QEMU.
#   .\run.ps1                # boots full GPT image with GUI window, networking & desktop
#   .\run.ps1 -Legacy        # boots bootstrap RAM-root
#   .\run.ps1 -Headless      # serial only (no GUI window)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$cargo = if ($env:CARGO) { $env:CARGO } else { "$env:USERPROFILE\.cargo\bin\cargo.exe" }
$qemu  = if ($env:QEMU)  { $env:QEMU }  else { "qemu-system-x86_64" }
$ovmf = if ($env:OVMF -and (Test-Path $env:OVMF)) {
    $env:OVMF
} else {
    $candidates = @(
        "C:\msys64\ucrt64\share\qemu\edk2-x86_64-code.fd",
        "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
        "C:\msys64\mingw64\share\qemu\edk2-x86_64-code.fd"
    )
    $found = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($found) { $found } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
}
$esp   = (Resolve-Path .\esp).Path

if (!(Get-Command zig -ErrorAction SilentlyContinue)) {
    $zigCandidates = @(
        "C:\Users\rocki\AppData\Local\Microsoft\WinGet\Packages\zig.zig_Microsoft.Winget.Source_8wekyb3d8bbwe\zig-x86_64-windows-0.16.0",
        (Get-ChildItem -Path "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Filter "zig.exe" -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty DirectoryName)
    )
    $zigDir = $zigCandidates | Where-Object { $_ -and (Test-Path (Join-Path $_ "zig.exe")) } | Select-Object -First 1
    if ($zigDir) {
        $env:PATH = "$zigDir;$env:PATH"
    }
}

Write-Host "[build] kernel"
& $cargo build -p vanta-kernel --target x86_64-unknown-none --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force -Path esp\boot | Out-Null
Copy-Item -Force target\x86_64-unknown-none\release\vanta-kernel esp\boot\vanta-kernel

if ($Gpt -and !$Legacy) {
    Write-Host "[build] GPT image"
    & $cargo xtask image
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if ($env:BUILD_ONLY) {
    Write-Host "[build] BUILD_ONLY set, done"
    exit 0
}

if ($Gpt -and !$Legacy) {
    $disk = (Resolve-Path .\target\vanta-gpt.img).Path
    $arguments = @(
        "-drive", "if=pflash,format=raw,readonly=on,file=$ovmf",
        "-drive", "file=$disk,if=none,format=raw,id=vd0",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0",
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0",
        "-serial", "stdio",
        "-smp", "2",
        "-m", "256M",
        "-no-reboot", "-no-shutdown"
    )
} else {
    $arguments = @(
        "-drive", "if=pflash,format=raw,readonly=on,file=$ovmf",
        "-drive", "format=raw,file=fat:rw:$esp,if=ide",
        "-serial", "stdio",
        "-smp", "2",
        "-m", "256M",
        "-no-reboot", "-no-shutdown"
    )
}

if ($Headless -or $env:HEADLESS) { $arguments += @("-display", "none") }

Write-Host "[run] $qemu (close the QEMU window to quit)"
if ($TimeoutSeconds -gt 0) {
    $proc = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -NoNewWindow
    Start-Sleep -Seconds $TimeoutSeconds
    if (!$proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
} else {
    & $qemu @arguments
}
