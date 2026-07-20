[CmdletBinding()]
param(
    [switch]$Virtio,
    [switch]$Network,
    [ValidateRange(5, 60)]
    [int]$TimeoutSeconds = 12
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$qemu = if ($env:QEMU) { $env:QEMU } else { "qemu-system-x86_64" }
$ovmf = if ($env:OVMF) { $env:OVMF } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
$esp = (Resolve-Path .\esp).Path
$log = Join-Path $env:TEMP "vanta-qemu-test.log"
$disk = Join-Path $env:TEMP "vanta-qemu-test-virtio.img"

$env:BUILD_ONLY = "1"
try {
    & .\run.ps1
    if ($LASTEXITCODE -ne 0) {
        throw "kernel build failed with exit code $LASTEXITCODE"
    }
} finally {
    Remove-Item Env:BUILD_ONLY -ErrorAction SilentlyContinue
}

Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$arguments = @(
    "-drive", "if=pflash,format=raw,readonly=on,file=`"$ovmf`"",
    "-drive", "format=raw,file=fat:rw:`"$esp`",if=ide",
    "-serial", "file:$log",
    "-smp", "2",
    "-m", "256M",
    "-no-reboot", "-no-shutdown", "-display", "none"
)

if ($Virtio) {
    Remove-Item -LiteralPath $disk -Force -ErrorAction SilentlyContinue
    $stream = [System.IO.File]::Open(
        $disk,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::None
    )
    try {
        $stream.SetLength(8MB)
    } finally {
        $stream.Dispose()
    }
    $arguments += @(
        "-drive", "file=$disk,if=none,format=raw,id=vd0",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0"
    )
}
if ($Network) {
    $arguments += @(
        "-netdev", "user,id=net0",
        "-device", "virtio-net-pci,disable-modern=on,ioeventfd=off,netdev=net0"
    )
}

function Invoke-QemuBoot {
    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Seconds $TimeoutSeconds
    } finally {
        if (!$process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
    }
    if (!(Test-Path -LiteralPath $log)) {
        throw "QEMU did not create a serial log"
    }
    Get-Content -LiteralPath $log -Raw
}

$output = Invoke-QemuBoot
$required = @(
    "[storage] writable VFS root mounted and lifecycle/remount self-check passed",
    "[smp] queued AP run queue=true dispatched=2",
    "[smp] AP cpu=1 run queue complete",
    "[shell] entering main loop"
)
if ($Virtio) {
    $required += "[storage] virtio-blk ready:"
}
if ($Network) {
    $required += "[net] udp dns reply"
}

foreach ($marker in $required) {
    if (!$output.Contains($marker)) {
        throw "QEMU regression failed: missing '$marker'`n$output"
    }
}

if ($Virtio) {
    $second_boot = Invoke-QemuBoot
    if (!$second_boot.Contains("[storage] persistent VFS mounted: existed=true")) {
        throw "QEMU persistence regression failed: disk was not remounted on the second boot`n$second_boot"
    }
}

Write-Host "[test] QEMU regression passed (virtio=$Virtio network=$Network)"
