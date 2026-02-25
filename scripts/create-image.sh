#!/usr/bin/env bash
set -euo pipefail

# Default to aarch64, allow override via ARCH env var or first argument
ARCH="${1:-${ARCH:-aarch64}}"

case "$ARCH" in
    aarch64)  alpine_arch="aarch64" ;;
    x86_64)   alpine_arch="x86_64" ;;
    *)        echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

# Error if mkfs.ext4 is not installed
if ! command -v mkfs.ext4 &> /dev/null; then
    echo "mkfs.ext4 could not be found. Please install e2fsprogs."
    exit 1
fi

# Error if wget is not installed
if ! command -v wget &> /dev/null; then
    echo "wget could not be found. Please install wget."
    exit 1
fi

# Error if jq is not installed
if ! command -v jq &> /dev/null; then
    echo "jq could not be found. Please install jq."
    exit 1
fi

base="$( cd "$( dirname "${BASH_SOURCE[0]}" )"/.. && pwd )"
pushd "$base" &>/dev/null || exit 1

img="$base/moss.img"

if [ -f "$img" ]; then
    rm "$img"
fi

touch "$img"
mkfs.ext4 "$img" 512M

# Download alpine minirootfs to $base/build/ if it doesn't exist
rootfs_tar="$base/build/alpine-minirootfs-${ARCH}.tar.gz"
if [ ! -f "$rootfs_tar" ]; then
    echo "Downloading alpine minirootfs for $ARCH..."
    mkdir -p "$base/build"
    wget -O "$rootfs_tar" "https://dl-cdn.alpinelinux.org/alpine/v3.23/releases/${alpine_arch}/alpine-minirootfs-3.23.3-${alpine_arch}.tar.gz"
fi

# Extract to directory $base/build/rootfs
if [ -d "$base/build/rootfs" ]; then
    rm -rf "$base/build/rootfs"
fi
mkdir -p build/rootfs
tar -xzf "$rootfs_tar" -C "$base/build/rootfs"

# Copy any extra binaries in $base/build/extra_bins to $base/build/rootfs/bin
if [ -d "$base/build/extra_bins" ]; then
    cp "$base/build/extra_bins/"* "$base/build/rootfs/bin/"
fi

# Build and copy over usertest
cd "$base"/usertest

# Determine target for cargo build
case "$ARCH" in
    aarch64)  cargo_target="aarch64-unknown-linux-musl" ;;
    x86_64)   cargo_target="x86_64-unknown-linux-musl" ;;
esac

echo "Building usertest for $ARCH..."
usertest_binary="$(cargo build --target "$cargo_target" --message-format=json | jq -r 'select(.reason == "compiler-artifact") | .filenames[]' | grep "usertest")"
cp "$usertest_binary" "$base/build/rootfs/bin/usertest"

# make image
yes | mkfs.ext4 -d "$base/build/rootfs" "$img" || true
