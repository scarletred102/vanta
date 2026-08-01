[CmdletBinding()]
param(
    [ValidateRange(5, 60)]
    [int]$TimeoutSeconds = 45
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$qemu = if ($env:QEMU) { $env:QEMU } else { "qemu-system-x86_64" }
$ovmf = if ($env:OVMF) { $env:OVMF } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
$log = Join-Path $env:TEMP "vanta-gpt-qemu-test.log"

cargo xtask image
if ($LASTEXITCODE -ne 0) {
    throw "GPT image build failed with exit code $LASTEXITCODE"
}

$image = (Resolve-Path .\target\vanta-gpt.img).Path
Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
$arguments = @(
    "-drive", "if=pflash,format=raw,readonly=on,file=`"$ovmf`"",
    "-drive", "file=`"$image`",if=none,format=raw,id=vd0",
    "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0",
    "-serial", "file:$log",
    "-smp", "2",
    "-m", "256M",
    "-no-reboot", "-no-shutdown", "-display", "none"
)

$process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
try {
    $required = @(
        "[storage] RedoxFS root mounted",
        "[storage] RedoxFS persistence check: true",
        "[proc] launching native /sbin/init",
        "[native] terminal/filesystem acceptance passed",
        "vanta native shell",
        "hello from C on Vanta",
        "libvanta SDK smoke passed",
        "libvanta stdio smoke passed"
    )
    $output = ""
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $log) {
            $output = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
            if ($null -eq $output) {
                $output = ""
            }
            if (($required | Where-Object { !$output.Contains($_) }).Count -eq 0) {
                Write-Host "[test] GPT native-init regression passed"
                exit 0
            }
        }
        if ($process.HasExited) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
    throw "GPT native-init regression failed. Serial log:`n$output"
} finally {
    if (!$process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
}
