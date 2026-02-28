# moss

![Architecture](https://img.shields.io/badge/arch-aarch64-blue)
![Language](https://img.shields.io/badge/language-Rust-orange)
![License](https://img.shields.io/badge/license-MIT-yellow)
[![IRC](https://img.shields.io/badge/OFTC_IRC-%23moss-blue)](https://webchat.oftc.net/?nick=&channels=%23moss)

![Moss Boot Demo](etc/moss_demo.gif)

**moss** is a Unix-like, Linux-compatible kernel written in Rust and AArch64
assembly.

It features an asynchronous kernel core, a modular architecture abstraction
layer, and binary compatibility with Linux userspace applications. Moss is
currently capable of running a dynamically linked Arch Linux AArch64 userspace,
including bash, BusyBox, coreutils, ps, top, and strace.

## Features

### Architecture & Memory
* Full support for AArch64.
* A well-defined HAL allowing for easy porting to other architectures (e.g.,
  x86_64, RISC-V).
* Memory Management:
    * Full MMU enablement and page table management.
    * Copy-on-Write (CoW) pages.
    * Safe copy to/from userspace async functions.
    * Kernel and userspace page fault management.
    * Kernel stack-overflow detection.
    * Shared library mapping and relocation.
    * `/proc/self/maps` support.
    * Buddy allocator for physical addresses and `smalloc` for boot allocations
      and tracking memory reservations.
    * A full slab allocator for kernel object allocations, featureing a per-CPU
      object cache.

### Async Core
One of the defining features of `moss` is its usage of Rust's `async/await`
model within the kernel context:
* All non-trivial system calls are written as `async` functions, sleep-able
  functions are prefixed with `.await`.
* The compiler enforces that spinlocks cannot be held over sleep points,
  eliminating a common class of kernel deadlocks.
* Any future can be wrapped with the `.interruptable()` combinator, allowing
  signals to interrupt the waiting future and appropriate action to be taken.

### Process Management
* Full task management including both UP and SMP scheduling via EEVDF and task
  migration via IPIs.
* Capable of running dynamically linked ELF binaries from Arch Linux.
* Currently implements [105 Linux syscalls](./etc/syscalls_linux_aarch64.md)
* `fork()`, `execve()`, `clone()`, and full process lifecycle management.
* Job control support (process groups, waitpid, background tasks).
* Signal delivery, masking, and propagation (SIGTERM, SIGSTOP, SIGCONT, SIGCHLD,
  etc.).
* ptrace support sufficient to run strace on Arch binaries.

### VFS & Filesystems
* Virtual File System with full async abstractions.
* Drivers:
    * Ramdisk block device implementation.
    * FAT32 filesystem driver (ro).
    * Ext2/3/4 filesystem driver (read support, partial write support).
    * `devfs` driver for kernel character device access.
    * `tmpfs` driver for temporary file storage in RAM (rw).
    * `procfs` driver for process and kernel information exposure.

## `libkernel` & Testing
`moss` is built on top of `libkernel`, a utility library designed to be
architecture-agnostic. This allows logic to be tested on a host machine (e.g.,
x86) before running on bare metal.

* Address Types: Strong typing for `VA` (Virtual), `PA` (Physical), and `UA`
  (User) addresses.
* Containers: `VMA` management, generic page-based ring buffer (`kbuf`), and
  waker sets.
* Sync Primitives: `spinlock`, `mutex`, `condvar`, `per_cpu`.
* Test Suite: A comprehensive suite of 230+ tests ensuring functionality across
  architectures (e.g., validating AArch64 page table parsing logic on an x86
  host).
* Userspace Testing, `usertest`: A dedicated userspace test-suite to validate
  syscall behavior in the kernel at run-time.

## Building and Running

### Prerequisites

You will need QEMU for AArch64 emulation, as well as wget, e2fsprogs, and jq for image creation.
We use `just` as a task runner to simplify common commands,
but you can also run the underlying commands directly if you prefer.

Additionally, you will need a version of
the [aarch64-none-elf](https://developer.arm.com/Tools%20and%20Software/GNU%20Toolchain) toolchain installed.

To install `aarch64-none-elf` on any OS, download the appropriate release of `aarch64-none-elf` onto your computer,
unpack it, then export the `bin` directory to PATH (Can be done via running:
`export PATH="~/Downloads/arm-gnu-toolchain-X.X.relX-x86_64-aarch64-none-elf/bin:$PATH"`, where X is the version number
you downloaded onto your machine, in your terminal).

#### Debian/Ubuntu
```bash
sudo apt install qemu-system-aarch64 wget jq e2fsprogs just
```

#### macOS

```bash
brew install qemu wget jq e2fsprogs just
```

#### NixOS

Run the following command:

```bash
nix shell nixpkgs#pkgsCross.aarch64-embedded.stdenv.cc nixpkgs#pkgsCross.aarch64-embedded.stdenv.cc.bintools
```

### Running via QEMU

### Building and booting the kernel

The process is identical for both supported architectures (x86_64 and
aarch64).  `moss.img` is an ext4 filesystem containing a minimal Alpine
minirootfs plus a custom `/bin/usertest` binary; the kernel mounts this image
as the root filesystem and executes the specified init program.

You can use `just` to handle all of the steps for you:

```bash
# create a new Alpine image for the default arch (x86_64) or pass `aarch64`
just create-image          # creates x86_64 image
just create-image aarch64  # builds the arm64 image

# build the kernel and launch it in QEMU
just run          # runs x86_64 kernel
just run aarch64  # runs the arm64 kernel
```

Both `qemu-runner` scripts accept additional command-line options that are
appended directly to the kernel command line.  This makes it easy to run a
one-shot command in the init shell without having to interact manually:

```bash
# start the x86_64 kernel, ask ash to print a message then sleep
./scripts/qemu-runner-x86_64.sh \
    target/x86_64-unknown-none/release/moss-kernel \
    --init=/bin/ash --init-arg=-c --init-arg='echo hello; sleep 5'
``` 

The same invocation works on `qemu-runner.sh` for aarch64.

If you don’t have `just` installed, the same thing can be performed manually:

```bash
# create the image (x86_64 by default, use ARCH or first argument to override)
./scripts/create-image.sh x86_64
# then run the kernel; an Alpine shell (`ash`/`sh`) is used by default
cargo run --release --target x86_64-unknown-none -- /bin/ash
```

The kernel will boot with `moss.img` as its initrd and will automatically
mount it using the `--rootfs=ext4fs` option.  The only difference between
architectures is the QEMU command line; the behaviour once the kernel has
started is the same.

### Running the Test Suite
Because `libkernel` is architecturally decoupled, you can run the logic tests on
your host machine:

``` bash
just test-unit
```

To run the userspace test suite in QEMU:

``` bash
just test-userspace
```

or

```bash
cargo run -r -- /bin/usertest
```

If you've made changes to the usertests and want to recreate the image, you can run:

``` bash
just create-image
```

or

```bash
./scripts/create-image.sh
```

### Roadmap & Status

moss is under active development. Current focus areas include:

* Networking Stack: TCP/IP implementation.
* A fully read/write capable filesystem driver.
* Expanding coverage beyond the current 105 calls.
* systemd bringup.

## Non-Goals (for now)

* Binary compatibility beyond AArch64.
* Production hardening.

Moss is an experimental kernel focused on exploring asynchronous design and
Linux ABI compatibility in Rust.

## Contributing

Contributions are welcome! Whether you are interested in writing a driver,
porting to x86, or adding syscalls.

## License
Distributed under the MIT License. See LICENSE for more information.
