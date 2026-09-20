//! x86_64 system call handling.
//!
//! syscall/sysret fast path for Linux ABI-compliant system calls.
//!
//! TODO(#16): Implement LSTAR MSR setup and syscall entry/exit.

/// System call numbers for x86_64 Linux ABI.
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

/// Set up the syscall entry point via the LSTAR MSR.
///
/// # Safety
///
/// Must be called once with interrupts disabled. Modifies the
/// IA32_LSTAR MSR.
pub unsafe fn setup_syscall_entry() {
    // TODO(#16):
    // 1. Write the syscall entry trampoline address to IA32_LSTAR MSR.
    // 2. Set IA32_FMASK to mask IF during syscall (0x200).
    // 3. Set IA32_EFER.SCE (bit 0) to enable syscall/sysret.
    // 4. Set STAR MSR for CS/SS selector base.
    todo!("setup_syscall_entry: syscall/sysret not yet implemented")
}
