#!/bin/bash
# ============================================================================
# VantaOS — ISO Creation Script
# Downloads Limine bootloader and creates a bootable ISO image.
# Prerequisites: git, xorriso (or xorrisofs), make
# ============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

KERNEL="zig-out/bin/vanta"
LIMINE_DIR="limine"
ISO_DIR="iso_root"
ISO_OUT="vanta.iso"

# ── Check prerequisites ──────────────────────────────────────────

if ! command -v xorriso &>/dev/null; then
    # Fallback: if an existing ISO exists, use the Python updater to hot-swap
    # the kernel binary into it (works on Windows without xorriso).
    if [ -f "$ISO_OUT" ] && command -v python3 &>/dev/null; then
        echo "xorriso not found — updating kernel in existing ISO via Python..."
        python3 "$SCRIPT_DIR/update_kernel_iso.py"
        if [ -f limine-bin/limine.exe ]; then
            ./limine-bin/limine.exe bios-install "$ISO_OUT" 2>/dev/null || true
        fi
        exit 0
    fi
    echo "ERROR: xorriso not found. Install it:"
    echo "  Ubuntu/Debian: sudo apt install xorriso"
    echo "  Fedora:        sudo dnf install xorriso"
    echo "  macOS:         brew install xorriso"
    echo "  Arch:          sudo pacman -S libisoburn"
    exit 1
fi

if [ ! -f "$KERNEL" ]; then
    echo "ERROR: Kernel not found at $KERNEL"
    echo "Run 'zig build' first."
    exit 1
fi

# ── Download Limine if needed ────────────────────────────────────

if [ ! -d "$LIMINE_DIR" ]; then
    echo "Downloading Limine bootloader..."
    git clone https://github.com/limine-bootloader/limine.git \
        --branch=v8.x-binary --depth=1
    make -C "$LIMINE_DIR"
fi

# ── Create ISO directory structure ───────────────────────────────

echo "Creating ISO structure..."
rm -rf "$ISO_DIR"
mkdir -p "$ISO_DIR/boot/limine"
mkdir -p "$ISO_DIR/EFI/BOOT"

# Copy kernel
cp "$KERNEL" "$ISO_DIR/boot/vanta"

# Copy Limine config
cp limine.conf "$ISO_DIR/boot/limine/"

# Copy Limine bootloader files
cp "$LIMINE_DIR/limine-bios.sys"    "$ISO_DIR/boot/limine/" 2>/dev/null || true
cp "$LIMINE_DIR/limine-bios-cd.bin" "$ISO_DIR/boot/limine/" 2>/dev/null || true
cp "$LIMINE_DIR/limine-uefi-cd.bin" "$ISO_DIR/boot/limine/" 2>/dev/null || true
cp "$LIMINE_DIR/BOOTX64.EFI"       "$ISO_DIR/EFI/BOOT/"    2>/dev/null || true
cp "$LIMINE_DIR/BOOTIA32.EFI"      "$ISO_DIR/EFI/BOOT/"    2>/dev/null || true

# ── Create ISO ───────────────────────────────────────────────────

echo "Creating ISO image..."
xorriso -as mkisofs \
    -b boot/limine/limine-bios-cd.bin \
    -no-emul-boot -boot-load-size 4 -boot-info-table \
    --efi-boot boot/limine/limine-uefi-cd.bin \
    -efi-boot-part --efi-boot-image --protective-msdos-label \
    "$ISO_DIR" -o "$ISO_OUT" 2>/dev/null

# Install Limine BIOS boot stages
if [ -f "$LIMINE_DIR/limine" ]; then
    "$LIMINE_DIR/limine" bios-install "$ISO_OUT" 2>/dev/null || true
fi

# ── Done ─────────────────────────────────────────────────────────

ISO_SIZE=$(du -h "$ISO_OUT" | cut -f1)
echo ""
echo "Created $ISO_OUT ($ISO_SIZE)"
echo "Run with: qemu-system-x86_64 -cdrom $ISO_OUT -serial stdio -m 256M"
