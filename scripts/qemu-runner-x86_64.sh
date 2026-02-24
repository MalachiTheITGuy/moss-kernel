#!/usr/bin/env sh

# Check if moss.img exists for x86_64 (though for -kernel we might not need it yet)
# But we'll follow the pattern.

qemu-system-x86_64 \
    -nographic \
    -serial mon:stdio \
    -cpu host \
    -enable-kvm \
    -m 1G \
    -kernel "$1" \
    -append "${@:2}" \
    -no-reboot
