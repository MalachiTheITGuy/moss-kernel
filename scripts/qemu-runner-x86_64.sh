#!/usr/bin/env bash
set -e

# First argument: the ELF file to run (required)
# All remaining arguments are treated as kernel command-line options and
# passed unmodified to `-append`.  This allows callers to specify an init
# binary plus arbitrary `--init-arg` flags, e.g.:
#
#   ./scripts/qemu-runner-x86_64.sh <kernel> --init=/bin/ash \
#       --init-arg=-c --init-arg='echo hello; sleep 5'
#
# If no extra args are provided we fall back to an interactive shell.

# Parse args:
if [ $# -lt 1 ]; then
    echo "Usage: $0 <elf-file> [<kernel-cmdline>...]"
    exit 1
fi

elf="$1"
shift

# Convenience: if the first remaining argument does *not* look like a
# long option, treat it as the init program path (the old behaviour).
# A bare path implies interactive mode, so --init-arg=-i is added automatically.
append_args=""
if [ $# -gt 0 ]; then
    if [[ "$1" != --* ]]; then
        # convert bare path into --init=... and force interactive mode
        append_args="--init=$1 --init-arg=-i"
        shift
    fi
    # append anything else verbatim
    if [ $# -gt 0 ]; then
        append_args="$append_args $*"
    fi
else
    # default to an interactive shell
    append_args="--init=/bin/sh --init-arg=-i"
fi

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"

# Note: x86_64 QEMU can accept the ELF directly, so we don’t actually need to
# produce a raw binary the way the aarch64 runner does.  The conversion step is
# left here as a no-op for now in case we start producing a non-ELF payload, but
# we deliberately use `$elf` below to avoid the "linux kernel too old to load a
# ram disk" error that arises when qemu is fed a raw dump.
# bin="${elf%.elf}.bin"

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

# Use nographic for better interactive serial handling
qemu-system-x86_64 \
    -M q35 \
    ${CPU_OPTS} \
    -m 2G \
    -smp 4 \
    -nographic \
    -serial mon:stdio \
    -kernel "$elf" \
    -initrd moss.img \
    -append "$append_args --rootfs=ext4fs --automount=/dev,devfs --automount=/tmp,tmpfs --automount=/proc,procfs --automount=/sys,sysfs" \
    ${KVM_OPTS}
