# VantaOS — Build and Run
# Usage: .\run.ps1 [-SkipBuild] [-SkipIso] [-NoDisplay]
param(
    [switch]$SkipBuild,
    [switch]$SkipIso,
    [switch]$NoDisplay
)

$ErrorActionPreference = "Stop"
$ZIG = "zig"
$PYTHON = "python"
$QEMU = "qemu-system-x86_64"
$ROOT = $PSScriptRoot

Write-Host ""
Write-Host "  ╔═══════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "  ║   VantaOS Build & Run                 ║" -ForegroundColor Cyan
Write-Host "  ╚═══════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

Set-Location $ROOT

# ── Step 1: Build kernel ─────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Host "[1/3] Building userspace servers..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Force -Path kernel/bin
    & $ZIG build-exe producer_stub.zig -T user.ld -fno-stack-check --name producer -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path producer -Destination kernel/bin/producer
    & $ZIG build-exe consumer_stub.zig -T user.ld -fno-stack-check --name consumer -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path consumer -Destination kernel/bin/consumer
    & $ZIG build-exe ahci_stub.zig -T user.ld -fno-stack-check --name ahci -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path ahci -Destination kernel/bin/ahci
    & $ZIG build-exe ns_stub.zig -T user.ld -fno-stack-check --name ns -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path ns -Destination kernel/bin/ns
    & $ZIG build-exe tmpfs_stub.zig -T user.ld -fno-stack-check --name tmpfs -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path tmpfs -Destination kernel/bin/tmpfs
    & $ZIG build-exe vantafs_stub.zig -T user.ld -fno-stack-check --name vantafs -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path vantafs -Destination kernel/bin/vantafs
    & $ZIG build-exe fs_test_stub.zig -T user.ld -fno-stack-check --name fs_test -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path fs_test -Destination kernel/bin/fs_test
    & $ZIG build-exe virtio_net_stub.zig -T user.ld -fno-stack-check --name virtio_net -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path virtio_net -Destination kernel/bin/virtio_net
    & $ZIG build-exe timer_stub.zig -T user.ld -fno-stack-check --name timer -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path timer -Destination kernel/bin/timer
    & $ZIG build-exe virtio_gpu_stub.zig -T user.ld -fno-stack-check --name virtio_gpu -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path virtio_gpu -Destination kernel/bin/virtio_gpu
    & $ZIG build-exe compositor_stub.zig -T user.ld -fno-stack-check --name compositor -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path compositor -Destination kernel/bin/compositor
    & $ZIG build-exe input_stub.zig -T user.ld -fno-stack-check --name input -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path input -Destination kernel/bin/input
    & $ZIG build-exe terminal_stub.zig -T user.ld -fno-stack-check --name terminal -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path terminal -Destination kernel/bin/terminal
    & $ZIG build-exe pty_stub.zig -T user.ld -fno-stack-check --name pty -O ReleaseSafe -target x86_64-freestanding-none
    Move-Item -Force -Path pty -Destination kernel/bin/pty
    Write-Host "      Userspace compiled successfully!" -ForegroundColor Green

    Write-Host "[1/3] Building kernel..." -ForegroundColor Yellow
    & $ZIG build
    if ($LASTEXITCODE -ne 0) { Write-Host "Build failed!" -ForegroundColor Red; exit 1 }
    $size = (Get-Item "zig-out\bin\vanta").Length / 1KB
    Write-Host "      Kernel: $([math]::Round($size)) KB" -ForegroundColor Green
} else {
    Write-Host "[1/3] Build skipped (-SkipBuild)" -ForegroundColor Gray
}

# ── Step 2: Create ISO ───────────────────────────────────────────
if (-not $SkipIso) {
    Write-Host "[2/3] Creating ISO..." -ForegroundColor Yellow
    & $PYTHON tools\build_iso.py
    if ($LASTEXITCODE -ne 0) { Write-Host "ISO creation failed!" -ForegroundColor Red; exit 1 }
} else {
    Write-Host "[2/3] ISO skipped (-SkipIso)" -ForegroundColor Gray
}

# ── Step 3: Launch QEMU ──────────────────────────────────────────
Write-Host "[3/3] Launching QEMU..." -ForegroundColor Yellow
Write-Host ""
Write-Host "  Serial output below (Ctrl+C to quit)" -ForegroundColor DarkGray
Write-Host "  ─────────────────────────────────────" -ForegroundColor DarkGray
Write-Host ""

$qemuArgs = @(
    "-cdrom", "vanta.iso",
    "-serial", "stdio",
    "-m", "256M",
    "-smp", "4",
    "-no-reboot",
    "-no-shutdown",
    "-device", "virtio-net-pci,netdev=n0",
    "-netdev", "user,id=n0",
    "-device", "virtio-gpu-pci"
)

if ($NoDisplay) {
    $qemuArgs += @("-nographic")
} else {
    $qemuArgs += @("-display", "sdl")
}

& $QEMU @qemuArgs
