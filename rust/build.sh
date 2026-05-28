#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

CARGO="${CARGO:-$USERPROFILE/.cargo/bin/cargo.exe}"
PROFILE="${PROFILE:-release}"
PROFILE_FLAG=""
PROFILE_DIR="debug"
if [ "$PROFILE" = "release" ]; then
    PROFILE_FLAG="--release"
    PROFILE_DIR="release"
fi

echo "[build] compiling kernel ($PROFILE)"
(cd kernel && "$CARGO" build $PROFILE_FLAG)

KERNEL_BIN="$(pwd)/kernel/target/x86_64-unknown-none/$PROFILE_DIR/vanta-kernel"
if [ ! -f "$KERNEL_BIN" ]; then
    echo "kernel binary not found: $KERNEL_BIN" >&2
    exit 1
fi
echo "[build] kernel: $KERNEL_BIN"

if [ "${BUILD_ONLY:-}" = "1" ]; then
    exit 0
fi

QEMU="${QEMU:-qemu-system-x86_64}"
QEMU_ARGS=(
    -kernel "$KERNEL_BIN"
    -serial stdio
    -m 256M
    -no-reboot
    -no-shutdown
    -d guest_errors
)
if [ "${HEADLESS:-}" = "1" ]; then
    QEMU_ARGS+=(-display none)
fi

echo "[run] $QEMU ${QEMU_ARGS[*]}"
exec "$QEMU" "${QEMU_ARGS[@]}"
