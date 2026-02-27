#!/usr/bin/env bash
set -e

# First argument: the ELF file to run (required)
# Second argument: init script to run (optional, defaults to /bin/sh)

# Parse args:
if [ $# -lt 1 ]; then
    echo "Usage: $0 <elf-file> [init-script]"
    exit 1
fi

if [ -n "$2" ]; then
    append_args="--init=$2"
else
    append_args="--init=/bin/sh --init-arg=-i"
fi

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

elf="$1"
bin="${elf%.elf}.bin"

# Convert to binary format
objcopy -O binary "$elf" "$bin"

# Enable KVM only when available
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
    -M q35 \
    ${CPU_OPTS} \
    -m 2G \
    -smp 4 \
    -display none \
    -monitor none \
    -serial stdio \
    -s \
    -kernel "$elf" \
    -initrd moss.img \
    -append "$append_args --rootfs=ext4fs --automount=/dev,devfs --automount=/tmp,tmpfs --automount=/proc,procfs --automount=/sys,sysfs" \
    ${KVM_OPTS}
