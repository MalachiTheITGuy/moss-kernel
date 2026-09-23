#!/usr/bin/env python3

"""
QEMU runner for moss-kernel x86_64 target.

Invoked automatically by `cargo run --target x86_64-unknown-none`.
Multiboot2 kernels are loaded directly as ELF — no objcopy conversion needed.

Usage (manual):
    python3 scripts/qemu_runner_x86_64.py target/x86_64-unknown-none/debug/moss
"""

import argparse
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(
        description="QEMU runner for moss-kernel x86_64"
    )

    parser.add_argument(
        "elf_executable",
        help="Path to the compiled ELF kernel",
    )

    # QEMU options
    parser.add_argument(
        "--memory",
        default="2G",
        help="Amount of guest memory (default: 2G)",
    )
    parser.add_argument(
        "--smp",
        default="4",
        help="Number of CPU cores (default: 4)",
    )
    parser.add_argument(
        "--cpu",
        default="qemu64",
        help="CPU model (default: qemu64)",
    )
    parser.add_argument(
        "--machine",
        default="q35",
        help="QEMU machine type (default: q35)",
    )
    parser.add_argument(
        "--debug",
        action="store_true",
        help="Halt at startup and wait for GDB connection on port 1234",
    )
    parser.add_argument(
        "--display",
        action="store_true",
        help="Add a display device to the VM (default: nographic)",
    )

    args = parser.parse_args()

    # ── Build QEMU command ──────────────────────────────────────────

    qemu_command = [
        "qemu-system-x86_64",
        f"-machine={args.machine}",
        f"-cpu={args.cpu}",
        f"-m={args.memory}",
        f"-smp={args.smp}",
        # Load Multiboot2 kernel directly from ELF
        "-kernel",
        args.elf_executable,
        # 16550 COM1 serial → stdio
        "-serial",
        "stdio",
        # Disable default display (serial-only)
        "-nographic",
        # Reset the CPU on triple-fault instead of rebooting (useful for debugging)
        "-device",
        "isa-debug-exit,iobase=0xf4,iosize=0x04",
        # QEMU debug monitor on localhost:1234 (only if --debug)
    ]

    if args.debug:
        # -S: halt at startup; -s: shorthand for GDB server on localhost:1234
        qemu_command.append("-S")
        qemu_command.append("-s")

    if args.display:
        # Remove -nographic, add a VGA device
        qemu_command = [c for c in qemu_command if c != "-nographic"]
        qemu_command += ["-device", "VGA"]

    # ── Execute ──────────────────────────────────────────────────────

    try:
        result = subprocess.run(qemu_command, check=True)
    except FileNotFoundError:
        print(
            "Error: qemu-system-x86_64 not found. "
            "Install QEMU: sudo apt install qemu-system-x86",
            file=sys.stderr,
        )
        sys.exit(1)
    except subprocess.CalledProcessError as e:
        sys.exit(e.returncode)


if __name__ == "__main__":
    main()
