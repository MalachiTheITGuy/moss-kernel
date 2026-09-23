//! x86_64 system call handling.
//!
//! syscall/sysret fast path for Linux ABI-compliant system calls.

use core::arch::asm;

use log::info;

use super::ExceptionState;

// ---------------------------------------------------------------------------
// MSR constants (Intel SDM Vol. 3, Table 2-2)
// ---------------------------------------------------------------------------

/// IA32_STAR — segment selector base for SYSCALL/SYSRET.
const MSR_STAR: u32 = 0xC000_0081;

/// IA32_LSTAR — SYSCALL entry point (RIP target).
const MSR_LSTAR: u32 = 0xC000_0082;

/// IA32_FMASK — RFLAGS mask applied on SYSCALL entry.
const MSR_FMASK: u32 = 0xC000_0084;

/// IA32_EFER — Extended Feature Enable Register.
const MSR_EFER: u32 = 0xC000_0080;

/// IA32_EFER.SCE — System Call Enable (bit 0).
const EFER_SCE: u64 = 1;

// ---------------------------------------------------------------------------
// Segment selector constants
//
// These match the GDT layout in `boot/gdt.rs` and the Linux ABI:
//   Index 1 (0x08) = kernel code  (DPL=0)
//   Index 2 (0x10) = kernel data  (DPL=0)
//   Index 5 (0x2B) = user data    (DPL=3, RPL=3)
//   Index 6 (0x33) = user code    (DPL=3, RPL=3)
// ---------------------------------------------------------------------------

/// Kernel code segment selector — GDT entry 1, DPL 0.
const KERNEL_CS: u64 = 0x08;

/// Kernel data segment selector — GDT entry 2, DPL 0.
const KERNEL_SS: u64 = 0x10;

/// User code segment selector — GDT entry 6, DPL 3, RPL 3.
const USER_CS: u64 = 0x33;

/// User data segment selector — GDT entry 5, DPL 3, RPL 3.
const USER_SS: u64 = 0x2B;

/// STAR MSR value encoding CS/SS selectors for SYSCALL and SYSRET.
///
/// Layout (Intel SDM Vol. 3, 2.8.1):
///   Bits [15:0]  = target CS  (loaded from STAR[47:32] + 16 on SYSCALL)
///   Bits [31:16] = target SS  (loaded from STAR[47:32] + 24 on SYSCALL)
///   Bits [47:32] = Syscall CS (user CS when entering kernel)
///   Bits [63:48] = Syscall SS (user SS when entering kernel)
///
/// For SYSCALL:  CS = STAR[47:32], SS = STAR[47:32] + 8
/// For SYSRET:   CS = STAR[31:16] + 16, SS = STAR[31:16] + 8
fn star_msr_value() -> u64 {
    (USER_SS << 48) | (USER_CS << 32) | (KERNEL_SS << 16) | KERNEL_CS
}

// ---------------------------------------------------------------------------
// Assembly entry point (defined in entry.S, no symbol mangling)
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn syscall_entry();
}

// ---------------------------------------------------------------------------
// Low-level MSR helpers
// ---------------------------------------------------------------------------

/// Write a 64-bit Model-Specific Register.
///
/// # Safety
///
/// Caller must ensure the MSR address is valid and that writing it
/// will not violate safety invariants.
unsafe fn write_msr(msr: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") low, in("edx") high);
    }
}

/// Read a 64-bit Model-Specific Register.
///
/// # Safety
///
/// Caller must ensure the MSR address is valid.
unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        asm!("rdmsr", out("eax") low, out("edx") high, in("ecx") msr);
    }
    ((high as u64) << 32) | (low as u64)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Set up the x86_64 SYSCALL/SYSRET fast-path entry point.
///
/// This programs four MSRs:
/// - **IA32_LSTAR**: the kernel entry point reached via `syscall`.
/// - **IA32_STAR**: segment selectors for ring transitions.
/// - **IA32_FMASK**: RFLAGS bits masked on entry (disables IF).
/// - **IA32_EFER.SCE**: enables the `syscall`/`sysret` instructions.
///
/// # Safety
///
/// Must be called exactly once with interrupts disabled.
/// Modifies MSRs that control privilege-level transitions.
pub unsafe fn setup_syscall_entry() {
    // SAFETY: Called once with interrupts disabled during boot.
    unsafe {
        // 1. Write the SYSCALL entry point address.
        write_msr(MSR_LSTAR, syscall_entry as *const () as u64);

        // 2. Program segment selectors for ring 0 ↔ ring 3 transitions.
        write_msr(MSR_STAR, star_msr_value());

        // 3. Mask IF (bit 9) on SYSCALL entry to prevent nested interrupts
        //    until the kernel explicitly re-enables them.
        write_msr(MSR_FMASK, 0x200);

        // 4. Enable the SYSCALL/SYSRET instructions via EFER.SCE (bit 0).
        //    Read-modify-write to preserve other EFER bits.
        let efer = read_msr(MSR_EFER);
        write_msr(MSR_EFER, efer | EFER_SCE);
    }

    info!(
        "syscall entry: LSTAR = {:#x}, STAR = {:#x}, FMASK = {:#x}",
        syscall_entry as *const () as u64,
        star_msr_value(),
        0x200u64,
    );
}

// ---------------------------------------------------------------------------
// Syscall numbers — x86_64 Linux ABI
// ---------------------------------------------------------------------------

/// System call numbers for x86_64 Linux ABI.
///
/// These match the x86_64 syscall table from Linux.
pub mod numbers {
    pub const SYS_READ: usize = 0;
    pub const SYS_WRITE: usize = 1;
    pub const SYS_OPEN: usize = 2;
    pub const SYS_CLOSE: usize = 3;
    pub const SYS_FSTAT: usize = 5;
    pub const SYS_MMAP: usize = 9;
    pub const SYS_MPROTECT: usize = 10;
    pub const SYS_MUNMAP: usize = 11;
    pub const SYS_BRK: usize = 12;
    pub const SYS_IOCTL: usize = 16;
    pub const SYS_SIGACTION: usize = 13;
    pub const SYS_RT_SIGACTION: usize = 13;
    pub const SYS_RT_SIGPROCMASK: usize = 14;
    pub const SYS_CLONE: usize = 56;
    pub const SYS_FORK: usize = 57;
    pub const SYS_VFORK: usize = 58;
    pub const SYS_EXECVE: usize = 59;
    pub const SYS_EXIT: usize = 60;
    pub const SYS_WAIT4: usize = 61;
    pub const SYS_KILL: usize = 62;
    pub const SYS_UNAME: usize = 63;
    pub const SYS_FCNTL: usize = 72;
    pub const SYS_GETPID: usize = 39;
    pub const SYS_GETPPID: usize = 110;
    pub const SYS_SOCKET: usize = 41;
    pub const SYS_GETTIMEOFDAY: usize = 96;
    pub const SYS_GETUID: usize = 102;
    pub const SYS_GETGID: usize = 104;
    pub const SYS_GETEUID: usize = 107;
    pub const SYS_GETEGID: usize = 108;
    pub const SYS_GETTID: usize = 186;
    pub const SYS_SET_TID_ADDRESS: usize = 218;
    pub const SYS_CLOCK_GETTIME: usize = 228;
    pub const SYS_FUTEX: usize = 202;
    pub const SYS_OPENAT: usize = 257;
    pub const SYS_MKDIRAT: usize = 258;
    pub const SYS_GETDENTS64: usize = 217;
    pub const SYS_UNLINKAT: usize = 263;
    pub const SYS_RENAMEAT: usize = 264;
    pub const SYS_FCHMODAT: usize = 268;
    pub const SYS_FCHOWNAT: usize = 260;
    pub const SYS_READLINKAT: usize = 267;
    pub const SYS_STATX: usize = 332;
    pub const SYS_RSEQ: usize = 354;
    pub const SYS_CLONE3: usize = 435;
    pub const SYS_CLOSE_RANGE: usize = 436;
    pub const SYS_FACCESSAT: usize = 269;
    pub const SYS_PIPE2: usize = 293;
    pub const SYS_DUP3: usize = 292;
    pub const SYS_NANOSLEEP: usize = 35;
    pub const SYS_EPOLL_CREATE1: usize = 291;
    pub const SYS_EPOLL_CTL: usize = 233;
    pub const SYS_EPOLL_WAIT: usize = 232;
    pub const SYS_UNLINK: usize = 87;
}

// ---------------------------------------------------------------------------
// x86_64 Linux errno values (subset used by stub dispatch)
// ---------------------------------------------------------------------------

/// ENOSYS — Function not implemented (x86_64 Linux value).
const ENOSYS: isize = -38;

// ---------------------------------------------------------------------------
// Syscall dispatch
// ---------------------------------------------------------------------------

/// Dispatch a syscall from the `syscall` instruction.
///
/// Called from `exception_dispatch` when `vector_num == SYSCALL (0x80)`.
/// The x86_64 Linux ABI passes arguments as:
///   - `rax` = syscall number
///   - `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` = arguments
///
/// On return, `state.rax` holds the result (or negative errno on error).
///
/// This is a stub implementation that logs the syscall number and returns
/// `ENOSYS`.  Individual syscalls will be implemented as part of later
/// phases (process management, filesystem, etc.).
pub fn syscall_dispatch(state: &mut ExceptionState) {
    let nr = state.rax;
    let arg1 = state.rdi;
    let arg2 = state.rsi;
    let arg3 = state.rdx;
    let arg4 = state.r10;
    let arg5 = state.r8;
    let arg6 = state.r9;

    // Dispatch individual syscalls.
    let result = match nr as usize {
        numbers::SYS_EXIT => sys_exit(state),
        numbers::SYS_WRITE => sys_write(arg1 as usize, arg2 as usize, arg3 as usize),
        _ => {
            // Trace unknown syscalls for development.
            info!(
                "syscall: unimplemented nr={} args=({:#x},{:#x},{:#x},{:#x},{:#x},{:#x})",
                nr, arg1, arg2, arg3, arg4, arg5, arg6,
            );
            Err(-ENOSYS)
        }
    };

    // Store the return value in RAX per the x86_64 ABI.
    state.rax = match result {
        Ok(val) => val as u64,
        Err(errno) => (errno as u64) & 0xFFFF_FFFF_FFFF_FFFF, // sign-extend for kernel errors
    };
}

// ---------------------------------------------------------------------------
// Individual syscall stubs
// ---------------------------------------------------------------------------

/// Exit the current task.
///
/// Mirrors the arm64 `sys_exit` stub: marks the state as finished so
/// the scheduler can reclaim the task.
fn sys_exit(state: &mut ExceptionState) -> Result<isize, isize> {
    info!("sys_exit: code={}", state.rdi);
    // TODO(#17): set TaskState::Finished, trigger reschedule.
    // For now, loop forever so we don't return to a dead task.
    loop {
        unsafe {
            asm!("hlt");
        }
    }
}

/// Write to a file descriptor (stub).
fn sys_write(_fd: usize, _buf: usize, _count: usize) -> Result<isize, isize> {
    info!("sys_write: fd={}, buf={:#x}, count={}", _fd, _buf, _count);
    // TODO(#19): implement via filesystem layer.
    Err(-ENOSYS)
}
