[CmdletBinding()]
param(
    [ValidateRange(5, 120)]
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
$manifest = (Resolve-Path .\target\vanta-gpt.manifest).Path
$imageHash = (Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash
$manifestHash = (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash
cargo xtask image
if ($LASTEXITCODE -ne 0) {
    throw "GPT reproducibility rebuild failed with exit code $LASTEXITCODE"
}
if ((Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash -ne $imageHash -or
    (Get-FileHash -LiteralPath $manifest -Algorithm SHA256).Hash -ne $manifestHash) {
    throw "GPT image reproducibility mismatch"
}
Write-Host "[test] GPT image reproducibility passed"

function Invoke-GptBoot {
    param(
        [Parameter(Mandatory)] [string]$DiskImage,
        [Parameter(Mandatory)] [string[]]$Required,
        [Parameter(Mandatory)] [string]$Label
    )

    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
    $arguments = @(
        "-drive", "if=pflash,format=raw,readonly=on,file=`"$ovmf`"",
        "-drive", "file=`"$DiskImage`",if=none,format=raw,id=vd0",
        "-device", "virtio-blk-pci,disable-modern=on,ioeventfd=off,drive=vd0",
        "-serial", "file:$log",
        "-smp", "2",
        "-m", "256M",
        "-no-reboot", "-no-shutdown", "-display", "none"
    )
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $output = ""
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            if (Test-Path -LiteralPath $log) {
                $output = Get-Content -LiteralPath $log -Raw -ErrorAction SilentlyContinue
                if ($null -eq $output) { $output = "" }
                if (($Required | Where-Object { !$output.Contains($_) }).Count -eq 0) {
                    Write-Host "[test] GPT $Label passed"
                    return $output
                }
            }
            if ($process.HasExited) { break }
            Start-Sleep -Milliseconds 100
        }
        throw "GPT $Label failed. Serial log:`n$output"
    } finally {
        if (!$process.HasExited) {
            Stop-Process -Id $process.Id -Force
        }
    }
}

$common = @(
    "[storage] RedoxFS root mounted",
    "[storage] RedoxFS persistence check: true",
    "[proc] launching native /sbin/init",
    "[native] acceptance: developer-gate ok",
    "[native] terminal/filesystem acceptance passed",
    "vanta native shell",
    "hello from C on Vanta",
    "libvanta SDK smoke passed",
    "libvanta stdio smoke passed",
    "libvanta directory smoke passed",
    "libvanta environment smoke passed",
    "libvanta process smoke passed",
    "[native] acceptance: c-exec-smoke ok",
    "[procd] service registered",
    "[procd] service upgraded",
    "[procd] service discovered",
    "[procd] vfs backend passed",
    "[procd] service authority revoked",
    "[native] acceptance: procd-gate ok",
    "[native] acceptance: audit-persistence ok",
    "[native] Gate B IPC acceptance passed"
)

$first = Invoke-GptBoot -DiskImage $image -Label "first boot" -Required ($common + "[storage] RedoxFS reboot persistence marker: false")
$second = Invoke-GptBoot -DiskImage $image -Label "reboot persistence" -Required ($common + "[storage] RedoxFS reboot persistence marker: true")

$corruptRoot = Join-Path $env:TEMP "vanta-gpt-corrupt-root.img"
Copy-Item -LiteralPath $image -Destination $corruptRoot -Force
$corruptLength = (Get-Item -LiteralPath $corruptRoot).Length - (2 * 1024 * 1024)
$stream = [IO.File]::Open($corruptRoot, [IO.FileMode]::Open, [IO.FileAccess]::Write, [IO.FileShare]::Read)
try {
    $stream.SetLength($corruptLength)
} finally {
    $stream.Dispose()
}
Invoke-GptBoot -DiskImage $corruptRoot -Label "corrupt-root recovery" -Required @(
    "[recovery] entering kernel recovery shell",
    "[shell] entering main loop"
) | Out-Null
Remove-Item -LiteralPath $corruptRoot -Force -ErrorAction SilentlyContinue

Write-Host "[test] GPT Gate A and Gate B native acceptance passed"
