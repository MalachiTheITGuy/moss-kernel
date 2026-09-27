#!/bin/bash
# build-initramfs.sh — Create an ext4 rootfs image with /sbin/init
#
# Usage: ./build-initramfs.sh
# Requires: /usr/sbin/mke2fs, mount/unmount (or unshare), the static init binary
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MKE2FS="/usr/sbin/mke2fs"
MOUNT_OPTS="-o loop"
IMG_SIZE_KB=16384   # 16 MiB
IMG_NAME="initramfs.img"
INIT_SRC="${SCRIPT_DIR}/init"

# Directories
WORK="${SCRIPT_DIR}/_work"
MNT="${WORK}/mnt"

cleanup() {
    if mountpoint -q "${MNT}" 2>/dev/null; then
        umount "${MNT}" 2>/dev/null || true
    fi
    rm -rf "${WORK}"
}
trap cleanup EXIT

# --- Build the init binary if needed ---
if [ ! -f "${INIT_SRC}" ]; then
    echo "[*] Compiling init.c → init (static)"
    gcc -static -O2 -o "${INIT_SRC}" "${SCRIPT_DIR}/init.c"
fi

echo "[*] init binary: $(file "${INIT_SRC}")"

# --- Create blank ext4 image ---
echo "[*] Creating ${IMG_SIZE_KB} KiB ext4 image"
rm -f "${SCRIPT_DIR}/${IMG_NAME}"
dd if=/dev/zero of="${SCRIPT_DIR}/${IMG_NAME}" bs=1K count=${IMG_SIZE_KB} 2>/dev/null
${MKE2FS} -t ext4 -F -q "${SCRIPT_DIR}/${IMG_NAME}" 2>/dev/null

# --- Mount and populate ---
echo "[*] Mounting image and populating rootfs"
mkdir -p "${MNT}"

# Try unshare mount (no sudo needed)
if command -v unshare &>/dev/null; then
    # Use FUSE-based ext4 mount if available, otherwise fall back
    unshare --mount -- bash -c "
        mount -o loop,ro '${SCRIPT_DIR}/${IMG_NAME}' '${MNT}' 2>/dev/null
        if [ \$? -ne 0 ]; then
            echo '[!] Cannot mount ext4 without root — using debugfs instead'
            exit 1
        fi
    " 2>/dev/null && MOUNTED=true || MOUNTED=false
else
    MOUNTED=false
fi

if [ "${MOUNTED}" = "false" ]; then
    echo "[*] No root access — using debugfs to populate the image"
    # Use debugfs to populate the image
    DEBUGFS="/usr/sbin/debugfs"
    if [ ! -x "${DEBUGFS}" ]; then
        echo "[!] debugfs not found at ${DEBUGFS}"
        echo "[!] Falling back to raw initramfs approach"
        exit 1
    fi

    # Create the directory structure using debugfs
    cat > "${WORK}/debugfs_cmds.txt" <<EOF
mkdir /sbin
mkdir /dev
mkdir /proc
mkdir /sys
mkdir /tmp
write ${INIT_SRC} /sbin/init
chmod 0755 /sbin/init
EOF

    ${DEBUGFS} -w -f "${WORK}/debugfs_cmds.txt" "${SCRIPT_DIR}/${IMG_NAME}" 2>/dev/null

    echo "[*] Verifying image contents:"
    ${DEBUGFS} -R "ls -l /sbin/" "${SCRIPT_DIR}/${IMG_NAME}" 2>/dev/null
else
    cp "${INIT_SRC}" "${MNT}/sbin/init"
    chmod 755 "${MNT}/sbin/init"
    mkdir -p "${MNT}"/{dev,proc,sys,tmp}
    umount "${MNT}"
fi

echo "[*] Rootfs image: ${SCRIPT_DIR}/${IMG_NAME}"
echo "[*] Size: $(du -k "${SCRIPT_DIR}/${IMG_NAME}" | cut -f1) KiB"
echo "[*] Done."
