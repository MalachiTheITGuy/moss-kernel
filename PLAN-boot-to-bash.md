# x86_64 Boot-to-Bash Implementation Plan

**Created**: 2026-10-25
**Goal**: Get moss-kernel x86_64 implementation to boot from QEMU through the complete
boot chain and land at a Bash shell.

## Current State Summary

| Layer | Status | Details |
|-------|--------|---------|
| **Boot** | ✅ Working | Multiboot2 → long mode → arch_init_stage1/2 → kmain() → exit(0) |
| **Memory** | ✅ Working | Page tables, heap, MMU, address space (with one todo) |
| **Exceptions** | ✅ Working | IDT, GDT, 256 ISR stubs, fault handlers |
| **Syscalls** | ⚠️ Stub only | 2 of ~80+ needed: `sys_exit` (hangs), `sys_write` (ENOSYS) |
| **Scheduling** | ✅ Working | Scheduler, task states, context switch, idle |
| **Process** | ✅ Working | ProcessCtx, ELF loading, exec, fork, signal, ptrace, vDSO |
| **Drivers** | ⚠️ Partial | LAPIC/IOAPIC, HPET, UART — but NO cmdline/initrd passthrough |
| **Filesystem** | ✅ Working (arch-agnostic) | VFS, ext4, FAT32, ramfs, devfs — all registered |
| **Console** | ❌ Blocked | `/dev/console` needs initrd + rootfs mount + TTY driver |

## The Boot-to-Bash Call Path

```
start.S → arch_init_stage1 → arch_init_stage2 → kmain(args, ctx_frame)
                                                       │
    ┌──────────────────────────────────────────────────┘
    ├── sched_init()
    ├── register_fs_drivers()          ← arch-agnostic, works
    ├── parse_args(args)               ← BROKEN: args is ""
    │     └── expects: "--init=/sbin/init --rootfs=ext4fs --automount=/dev,devfs"
    ├── spawn_kernel_work(launch_init(ctx, kopts))
    │     ├── initrd_block_dev         ← BROKEN: None for x86_64
    │     ├── VFS.mount_root()         ← needs rootfs driver
    │     ├── VFS.open("/dev/console") ← needs devfs mounted
    │     ├── set stdin/stdout/stderr
    │     └── kernel_exec()            ← loads ELF, sets up user stack
    └── dispatch_userspace_task(ctx_frame)  ← BROKEN: null pointer
```

## 6 Critical Gaps (in dependency order)

### Gap 1: Multiboot2 Cmdline Passthrough
**File**: `src/arch/x86_64/boot/mod.rs`
**Problem**: `parse_cmdline()` at line 204 parses the tag correctly but the result
is discarded. `arch_init_stage2()` passes `String::new()` to kmain.
`get_cmdline()` returns `""`.
**Fix**:
- Add BSS statics: `BOOT_CMDLINE: [u8; 512]` + `BOOT_CMDLINE_LEN: AtomicUsize`
- In `arch_init_stage1`, after parsing Multiboot2 tags, store raw cmdline bytes
- In `arch_init_stage2`, reconstruct `String` via `load_cmdline_string()`
- Pattern from x86_64-dev branch (lines 201-223)
**Complexity**: Low (~40 lines)

### Gap 2: Multiboot2 Initrd/Module Tag Parsing
**File**: `src/arch/x86_64/boot/mod.rs` + `src/main.rs`
**Problem**: No Multiboot2 module tag parsing. `initrd_block_dev` is `None` for x86_64.
**Fix**:
- In `arch_init_stage1`, iterate Multiboot2 tags to find MODULE tag (type 3)
- Store initrd physical start/end in BSS statics
- In `launch_init()`, replace `#[cfg(target_arch = "x86_64")]` block with actual
  `RamdiskBlkDev` creation
- Use x86_64 direct mapping VA (`0xFFFF_8880_0000_0000`), not ARM's `0xffff_9800_0000_0000`
**Complexity**: Medium (~80 lines)

### Gap 3: Initial Userspace Context
**File**: `src/arch/x86_64/boot/mod.rs` (line 398)
**Problem**: `kmain()` receives `ctx_frame: *mut UserCtx = null_mut()`.
**Fix**:
```rust
let mut initial_ctx: crate::process::ctx::UserCtx = unsafe { core::mem::zeroed() };
crate::kmain(cmdline_str, &mut initial_ctx as *mut _);
```
**Complexity**: Low (~5 lines)

### Gap 4: address_space::unmap Implementation
**File**: `src/arch/x86_64/memory/address_space.rs:85`
**Problem**: `unmap()` is `todo!()`.
**Fix**: Implement using `walk_and_modify_region` with `PTE::invalid()`.
**Complexity**: Low (~15 lines)

### Gap 5: Syscall Dispatch (THE BIG ONE)
**File**: `src/arch/x86_64/exceptions/syscall.rs`
**Problem**: Only 2 stub syscalls. Need ~80+ for bash.

#### Phase 5a: Core Syscalls (20 essential for minimal shell)

| x86_64 NR | Name | Why Needed |
|-----------|------|------------|
| 0 | read | stdin reads |
| 1 | write | stdout/stderr |
| 2 | open | open files |
| 3 | close | close fds |
| 5 | fstat | stat by fd |
| 9 | mmap | ELF loading, brk |
| 10 | mprotect | ELF segments |
| 11 | munmap | cleanup |
| 12 | brk | heap allocation |
| 16 | ioctl | TTY control |
| 39 | getpid | process info |
| 56 | clone | fork/thread |
| 57 | fork | process fork |
| 59 | execve | exec binary |
| 60 | exit | terminate process |
| 61 | wait4 | wait for child |
| 63 | uname | kernel version |
| 228 | clock_gettime | timing |
| 231 | exit_group | process exit |
| 257 | openat | open files (alt) |

#### Phase 5b: Essential Supporting Syscalls (~30 more)
rt_sigaction, rt_sigprocmask, fcntl, getcwd, chdir, getdents64,
fchmodat, faccessat, pipe2, dup3, prlimit64, getrandom, statx, etc.

#### Phase 5c: Syscall Architecture Bridge
x86_64 dispatches synchronously but many VFS ops are async.
**Recommended**: Use `pollster::block_on` in syscall handler:
```rust
pub fn syscall_dispatch(state: &mut ExceptionState) {
    let result = pollster::block_on(async {
        match nr { /* ... */ }
    });
    state.rax = result_to_errno(result);
}
```

**Complexity**: Very High (~1500-2000 lines)

### Gap 6: TTY/Console Driver
**File**: New + adaptation of 16550 UART
**Problem**: `/dev/console` needs TTY driver bridging UART ↔ devfs.
**Fix**: Create TTY layer over 16550 UART for QEMU serial console.
**Complexity**: Medium-High (~300-500 lines)

## Implementation Order

```
Phase 5a: Infrastructure Fixes (Gaps 1-4) ← Independent, parallel PRs OK
├── Gap 3: Initial ctx_frame fix (5 lines)
├── Gap 4: unmap implementation (15 lines)
├── Gap 1: Cmdline passthrough (40 lines)
└── Gap 2: Initrd module parsing (80 lines)

Phase 5b: Minimal Syscall Dispatch (Gap 5a) ← Sequential
├── Refactor syscall_dispatch for async/sync bridge
├── Port 20 core syscalls from arm64
└── Test: kernel_exec loads ELF, shell starts

Phase 5c: Console & TTY (Gap 6) ← Sequential
├── TTY driver for 16550 UART
├── devfs /dev/console registration
└── Test: shell prompt appears, basic I/O works

Phase 5d: Essential Syscalls (Gap 5b) ← Sequential
├── Port 30 supporting syscalls
└── Test: pipes, signals, directory ops work

Phase 5e: Bash Verification ← Final
├── Build minimal initramfs with busybox/musl-static
├── Boot QEMU → GRUB → kernel → init → /bin/sh
└── Verify: prompt, basic commands, exit
```

## Reference: x86_64-dev Branch

The fork's `origin/x86_64-dev` branch has useful concepts (NOT to merge directly):
- cmdline storage: `BOOT_CMDLINE: [u8; 512]` + `AtomicUsize`
- initrd module handling: `HvmStartInfo`/`HvmModlist` structs
- initial ctx pattern: `core::mem::zeroed()` UserCtx
- serial setup: `setup_serial()` with interrupt claiming

⚠️ Branch is 326 files diverged from master. Extract concepts only.

## Estimated Effort

| Phase | Lines | Complexity | PRs |
|-------|-------|-----------|-----|
| 5a: Infrastructure | ~140 | Low-Medium | 1 |
| 5b: Core Syscalls | ~800 | High | 1-2 |
| 5c: Console/TTY | ~400 | Medium-High | 1 |
| 5d: Full Syscalls | ~800 | Medium | 1-2 |
| **Total** | **~2,140** | | **4-6 PRs** |

## Key Risks

1. **Async/Sync bridge** — block_on in syscall handler vs full async refactor
2. **ELF loading** — needs working mmap/brk + address space ops
3. **Initramfs format** — ARM uses raw cpio; verify x86_64 GRUB module compatibility
4. **Musl-libc compatibility** — iterative debugging with unimplemented syscall tracing
