#!/bin/bash
# ============================================================================
# VantaOS — Build & Run in QEMU
# Builds the kernel, creates an ISO, and launches QEMU with serial output.
# ============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

echo "╔═══════════════════════════════════════╗"
echo "║   VantaOS Build & Run                 ║"
echo "╚═══════════════════════════════════════╝"
echo ""

# ── Build kernel ─────────────────────────────────────────────────
echo "[1/3] Building kernel..."
zig build

# ── Create ISO ───────────────────────────────────────────────────
echo "[2/3] Creating ISO..."
bash "$SCRIPT_DIR/build-iso.sh"

# ── Launch QEMU ──────────────────────────────────────────────────
echo "[3/3] Launching QEMU..."
echo ""
echo "════════════════ QEMU Serial Output ════════════════"
echo ""

qemu-system-x86_64 \
    -cdrom vanta.iso \
    -serial stdio \
    -m 256M \
    -no-reboot \
    -no-shutdown \
    -display sdl \
    2>/dev/null

# If SDL display isn't available, try without display
# qemu-system-x86_64 -cdrom vanta.iso -serial stdio -m 256M -no-reboot -nographic
