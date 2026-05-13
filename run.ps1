# VantaOS — Build and Run
# Usage: .\run.ps1 [-SkipBuild] [-SkipIso] [-NoDisplay]
param(
    [switch]$SkipBuild,
    [switch]$SkipIso,
    [switch]$NoDisplay
)

$ErrorActionPreference = "Stop"
$ZIG = "C:\Users\LaLa\AppData\Roaming\Python\Python314\Scripts\python-zig.exe"
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
    "-no-reboot",
    "-no-shutdown"
)

if ($NoDisplay) {
    $qemuArgs += @("-nographic")
} else {
    $qemuArgs += @("-display", "sdl")
}

& $QEMU @qemuArgs
