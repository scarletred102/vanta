# Build the vanta kernel and boot in QEMU.
#   .\run.ps1                # graphical
#   $env:HEADLESS=1; .\run.ps1   # serial only
#   $env:BUILD_ONLY=1; .\run.ps1 # build only
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$cargo = if ($env:CARGO) { $env:CARGO } else { "$env:USERPROFILE\.cargo\bin\cargo.exe" }
$qemu  = if ($env:QEMU)  { $env:QEMU }  else { "qemu-system-x86_64" }
$ovmf  = if ($env:OVMF)  { $env:OVMF }  else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
$esp   = (Resolve-Path .\esp).Path

Write-Host "[build] kernel"
Push-Location kernel
& $cargo build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; exit $LASTEXITCODE }
Pop-Location
Copy-Item -Force kernel\target\x86_64-unknown-none\release\vanta-kernel esp\boot\vanta-kernel

if ($env:BUILD_ONLY) {
    Write-Host "[build] BUILD_ONLY set, done"
    exit 0
}

$args = @(
    "-drive", "if=pflash,format=raw,readonly=on,file=$ovmf",
    "-drive", "format=raw,file=fat:rw:$esp,if=ide",
    "-serial", "stdio",
    "-m", "256M",
    "-no-reboot", "-no-shutdown"
)
if ($env:HEADLESS) { $args += @("-display", "none") }

Write-Host "[run] $qemu (close the QEMU window to quit)"
& $qemu @args
