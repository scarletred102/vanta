#!/usr/bin/env bash
# Build the vanta kernel and boot it in QEMU.
#   ./run.sh              # build + boot with a graphical window
#   HEADLESS=1 ./run.sh   # serial only, no display
#   BUILD_ONLY=1 ./run.sh # build only
set -euo pipefail
cd "$(dirname "$0")"

CARGO="${CARGO:-$USERPROFILE/.cargo/bin/cargo.exe}"
QEMU="${QEMU:-qemu-system-x86_64}"
OVMF="${OVMF:-C:/Program Files/qemu/share/edk2-x86_64-code.fd}"
ESP_PATH="$(pwd -W 2>/dev/null || pwd)/esp"

echo "[build] kernel"
"$CARGO" build -p vanta-kernel --target x86_64-unknown-none --release
cp target/x86_64-unknown-none/release/vanta-kernel esp/boot/vanta-kernel

if [ "${BUILD_ONLY:-}" = "1" ]; then
    echo "[build] BUILD_ONLY set, exiting"
    exit 0
fi

ARGS=(
    -drive "if=pflash,format=raw,readonly=on,file=$OVMF"
    -drive "format=raw,file=fat:rw:$ESP_PATH,if=ide"
    -serial stdio
    -m 256M
    -no-reboot -no-shutdown
)
if [ "${HEADLESS:-}" = "1" ]; then
    ARGS+=(-display none)
fi

echo "[run] qemu-system-x86_64 (close the QEMU window to quit)"
exec "$QEMU" "${ARGS[@]}"
