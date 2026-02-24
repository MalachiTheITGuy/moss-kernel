#!/usr/bin/env sh

# Enable KVM only when available (to avoid qemu failing on systems without KVM)
KVM_OPTS=""
if [ "$(uname -s)" = "Linux" ] && [ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    KVM_OPTS="-enable-kvm"
fi

qemu-system-x86_64 \
    -nographic \
    -serial mon:stdio \
    -cpu host \
    ${KVM_OPTS} \
    -m 1G \
    -kernel "$1" \
    -append "${@:2}" \
    -no-reboot
