#!/usr/bin/env sh

# Enable KVM only when available (to avoid qemu failing on systems without KVM)
KVM_OPTS=""
if [ "$(uname -s)" = "Linux" ] && [ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    KVM_OPTS="-enable-kvm"
fi

# Use host CPU model only when KVM is enabled; fall back to a generic emulated CPU otherwise
CPU_OPTS="-cpu qemu64"
if [ -n "${KVM_OPTS}" ]; then
    CPU_OPTS="-cpu host"
fi

qemu-system-x86_64 \
    -nographic \
    -serial mon:stdio \
    ${CPU_OPTS} \
    ${KVM_OPTS} \
    -m 1G \
    -kernel "$1" \
    -append "${@:2}" \
    -no-reboot
