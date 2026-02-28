#!/usr/bin/env bash
set -e

# First argument: the ELF file to run (required)
# All remaining arguments are passed directly as kernel command line
# parameters (same semantics as the x86_64 runner above).

if [ $# -lt 1 ]; then
    echo "Usage: $0 <elf-file> [<kernel-cmdline>...]"
    exit 1
fi

elf="$1"
shift

# Same convenience as the x86_64 runner.
# A bare path implies interactive mode, so --init-arg=-i is added automatically.
append_args=""
if [ $# -gt 0 ]; then
    if [[ "$1" != --* ]]; then
        append_args="--init=$1 --init-arg=-i"
        shift
    fi
    if [ $# -gt 0 ]; then
        append_args="$append_args $*"
    fi
else
    append_args="--init=/bin/sh --init-arg=-i"
fi


base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"
bin="${elf%.elf}.bin"

# Convert to binary format
aarch64-none-elf-objcopy -O binary "$elf" "$bin"
qemu-system-aarch64 -M virt,gic-version=3 -initrd moss.img -cpu cortex-a72 -m 2G -smp 4 -nographic -s -kernel "$bin" -append "$append_args --rootfs=ext4fs --automount=/dev,devfs --automount=/tmp,tmpfs --automount=/proc,procfs --automount=/sys,sysfs"
