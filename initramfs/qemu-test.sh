#!/bin/bash
# qemu-test.sh — Boot moss-kernel in QEMU and capture serial output
#
# Usage: ./qemu-test.sh [initramfs.img]
#
# Requires: qemu-system-x86_64, the moss kernel ELF, initramfs.img
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL="${SCRIPT_DIR}/../target/x86_64-unknown-none/debug/moss"
INITRD="${1:-${SCRIPT_DIR}/initramfs.img}"
TIMEOUT_SECS=15

# Check prerequisites
if [ ! -f "${KERNEL}" ]; then
    echo "[!] Kernel not found: ${KERNEL}"
    echo "[!] Run 'cargo build' first."
    exit 1
fi

if [ ! -f "${INITRD}" ]; then
    echo "[!] Initrd not found: ${INITRD}"
    echo "[!] Run build-initramfs.sh first."
    exit 1
fi

KERNEL_SIZE=$(stat -c%s "${KERNEL}" 2>/dev/null || stat -f%z "${KERNEL}")
INITRD_SIZE=$(stat -c%s "${INITRD}" 2>/dev/null || stat -f%z "${INITRD}")
echo "[*] Kernel:  ${KERNEL} (${KERNEL_SIZE} bytes)"
echo "[*] Initrd:  ${INITRD} (${INITRD_SIZE} bytes)"

# Kernel command line:
#   rootfs=ext4fs   — mount root via ext4 driver
#   init=/sbin/init — first userspace process
CMDLINE="rootfs=ext4fs init=/sbin/init"

echo "[*] Kernel cmdline: ${CMDLINE}"
echo "[*] Booting QEMU (timeout: ${TIMEOUT_SECS}s)..."
echo "============================================"

# Run QEMU with:
#   -kernel:     PVH/ELF boot (our kernel)
#   -initrd:     Pass initramfs as Multiboot2 module or PVH module
#   -append:     Kernel command line
#   -nographic:  Serial console on stdio
#   -m 256M:     RAM (keep small for speed)
#   -s:          GDB stub on port 1234
#   -d int,cpu_reset,guest_errors: QEMU debug output
#   -D /tmp/qemu.log: QEMU log file
#   -no-reboot:  Don't reboot on triple fault
#   -serial stdio: Redirect serial to terminal
#   -monitor none: Disable QEMU monitor

QEMU_LOG="/tmp/moss-qemu.log"

# Trap to kill QEMU on exit
QEMU_PID=""
cleanup() {
    if [ -n "${QEMU_PID}" ] && kill -0 "${QEMU_PID}" 2>/dev/null; then
        kill "${QEMU_PID}" 2>/dev/null || true
        wait "${QEMU_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

timeout "${TIMEOUT_SECS}" qemu-system-x86_64 \
    -kernel "${KERNEL}" \
    -initrd "${INITRD}" \
    -append "${CMDLINE}" \
    -nographic \
    -m 256M \
    -s \
    -d int,cpu_reset,guest_errors \
    -D "${QEMU_LOG}" \
    -no-reboot \
    -serial stdio \
    -monitor none \
    -no-reboot \
    2>&1 &
QEMU_PID=$!

# Wait for QEMU to finish (or timeout)
set +e
wait "${QEMU_PID}" 2>/dev/null
QEMU_EXIT=$?
set -e

echo ""
echo "============================================"
echo "[*] QEMU exited with code: ${QEMU_EXIT}"
if [ -f "${QEMU_LOG}" ]; then
    echo "[*] QEMU debug log (last 30 lines):"
    tail -30 "${QEMU_LOG}"
fi
echo "============================================"
