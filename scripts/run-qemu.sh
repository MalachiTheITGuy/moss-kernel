#!/bin/bash
# Run moss-kernel x86_64 under QEMU for Multiboot2 boot validation
KERNEL="${1:-target/x86_64-unknown-none/debug/moss-kernel}"

if [ ! -f "$KERNEL" ]; then
    echo "Error: Kernel not found at $KERNEL"
    echo "Build with: cargo build --target x86_64-unknown-none"
    exit 1
fi

exec qemu-system-x86_64 \
    -machine q35 \
    -cpu qemu64 \
    -serial stdio \
    -m 2G \
    -smp 4 \
    -kernel "$KERNEL"
