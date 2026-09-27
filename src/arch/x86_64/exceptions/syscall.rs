//! x86_64 system call handling.
//!
//! syscall/sysret fast path for Linux ABI-compliant system calls.
//! The `handle_syscall` async function is the main dispatch, mirroring
//! the ARM64 implementation in `arch/arm64/exceptions/syscall.rs`.

use core::arch::asm;

use log::info;

use crate::{
    clock::syscalls::{
        gettime::sys_clock_gettime,
        itimer::{sys_getitimer, sys_setitimer},
        settime::sys_clock_settime,
        timeofday::sys_gettimeofday,
    },
    fs::{
        dir::sys_getdents64,
        memfd::sys_memfd_create,
        pipe::sys_pipe2,
        syscalls::{
            at::{
                access::{sys_faccessat, sys_faccessat2},
                chmod::sys_fchmodat,
                chown::sys_fchownat,
                link::sys_linkat,
                mkdir::sys_mkdirat,
                open::sys_openat,
                readlink::sys_readlinkat,
                rename::{sys_renameat, sys_renameat2},
                stat::sys_newfstatat,
                statx::sys_statx,
                symlink::sys_symlinkat,
                unlink::sys_unlinkat,
                utime::sys_utimensat,
            },
            chdir::{sys_chdir, sys_chroot, sys_fchdir, sys_getcwd},
            chmod::sys_fchmod,
            chown::sys_fchown,
            close::{sys_close, sys_close_range},
            copy_file_range::sys_copy_file_range,
            ioctl::sys_ioctl,
            iov::{sys_preadv, sys_preadv2, sys_pwritev, sys_pwritev2, sys_readv, sys_writev},
            mount::sys_mount,
            rw::{sys_pread64, sys_pwrite64, sys_read, sys_write},
            seek::sys_lseek,
            splice::sys_sendfile,
            statfs::{sys_fstatfs, sys_statfs},
            sync::sys_syncfs,
            trunc::sys_ftruncate,
        },
    },
    kernel::{
        hostname::sys_sethostname, power::sys_reboot, rand::sys_getrandom, sysinfo::sys_sysinfo,
        uname::sys_uname,
    },
    memory::{
        brk::sys_brk,
        mincore::sys_mincore,
        mmap::{sys_mmap, sys_mprotect, sys_mremap, sys_munmap},
        process_vm::sys_process_vm_readv,
    },
    net::syscalls::{
        accept::{sys_accept, sys_accept4},
        bind::sys_bind,
        connect::sys_connect,
        listen::sys_listen,
        recv::sys_recvfrom,
        send::sys_sendto,
        shutdown::sys_shutdown,
        socket::sys_socket,
    },
    process::{
        caps::{sys_capget, sys_capset},
        clone::sys_clone,
        creds::{
            sys_getegid, sys_geteuid, sys_getgid, sys_gettid, sys_getuid, sys_setfsgid,
            sys_setfsuid, sys_setgid, sys_setregid, sys_setresgid, sys_setresuid, sys_setreuid,
            sys_setuid,
        },
        epoll::{sys_epoll_create1, sys_epoll_ctl, sys_epoll_pwait},
        exec::sys_execve,
        exit::{sys_exit, sys_exit_group},
        fd_table::{
            dup::{sys_dup, sys_dup3},
            fcntl::sys_fcntl,
            select::{sys_ppoll, sys_pselect6},
        },
        inotify::{sys_inotify_add_watch, sys_inotify_init1, sys_inotify_rm_watch},
        pidfd::sys_pidfd_open,
        prctl::sys_prctl,
        ptrace::{TracePoint, ptrace_stop, sys_ptrace},
        sleep::{sys_clock_nanosleep, sys_nanosleep},
        thread_group::{
            Pgid,
            pid::{sys_getpgid, sys_getpid, sys_getppid, sys_setpgid},
            rsrc_lim::sys_prlimit64,
            signal::{
                kill::{sys_kill, sys_tkill},
                sigaction::sys_rt_sigaction,
                sigaltstack::sys_sigaltstack,
                signalfd::sys_signalfd4,
                sigprocmask::sys_rt_sigprocmask,
            },
            umask::sys_umask,
            wait::{sys_wait4, sys_waitid},
        },
        threading::{futex::sys_futex, sys_set_robust_list, sys_set_tid_address},
    },
    sched::{
        self,
        sched_task::state::TaskState,
        syscalls::{sys_sched_getaffinity, sys_sched_setaffinity, sys_sched_yield},
    },
};
use libkernel::{
    error::syscall_error::kern_err_to_syscall,
    memory::address::{TUA, UA, VA},
};

use crate::sched::syscall_ctx::ProcessCtx;

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
/// # Safety
///
/// Must be called exactly once with interrupts disabled.
pub unsafe fn setup_syscall_entry() {
    unsafe {
        write_msr(MSR_LSTAR, syscall_entry as *const () as u64);
        write_msr(MSR_STAR, star_msr_value());
        write_msr(MSR_FMASK, 0x200);
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
// System call dispatch (async)
//
// Mirrors the ARM64 `handle_syscall` in `arch/arm64/exceptions/syscall.rs`.
// Uses x86_64 Linux ABI syscall numbers.
//
// x86_64 Linux ABI:
//   rax = syscall number
//   rdi, rsi, rdx, r10, r8, r9 = arguments
//   rax = return value (or negative errno)
// ---------------------------------------------------------------------------

/// Dispatch a system call from the `syscall` instruction.
///
/// Called via `spawn_kernel_work` from `x86_64_interrupt_handler` when
/// `vector_num == SYSCALL`.  Reads the syscall number and arguments from
/// the saved user register context (`X86_64PtraceGPRegs`), dispatches to
/// the appropriate async implementation, and writes the result back to
/// `rax` in the user context.
pub async fn handle_syscall(mut ctx: ProcessCtx) {
    ctx.task_mut().update_accounting(None);
    ctx.task_mut().in_syscall = true;
    ptrace_stop(&ctx, TracePoint::SyscallEntry).await;

    // Extract syscall number and arguments from the saved user context.
    // x86_64 Linux ABI: rax = syscall number,
    //   rdi, rsi, rdx, r10, r8, r9 = arguments.
    let (nr, arg1, arg2, arg3, arg4, arg5, arg6) = {
        let state = ctx.task().ctx.user();
        (
            state.rax as u32,
            state.rdi,
            state.rsi,
            state.rdx,
            state.r10,
            state.r8,
            state.r9,
        )
    };

    // x86_64 Linux syscall numbers — must match the kernel headers exactly.
    //
    // Reference: arch/x86/entry/syscalls/syscall_64.tbl
    // Each number appears exactly once.
    let res = match nr as usize {
        // ── File I/O (Linux x86_64 numbers) ─────────────────────
        0x00 => sys_read(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x01 => sys_write(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x02 => {
            sys_openat(
                &ctx,
                (-100_i64 as u64).into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        } // open → openat(AT_FDCWD)
        0x03 => sys_close(&ctx, arg1.into()).await,
        0x05 => {
            sys_newfstatat(
                &ctx,
                (-100_i64 as u64).into(),
                TUA::from_value(arg1 as _),
                TUA::from_value(arg2 as _),
                0,
            )
            .await
        } // stat → newfstatat(AT_FDCWD)
        0x08 => sys_lseek(&ctx, arg1.into(), arg2 as _, arg3 as _).await,
        0x10 => sys_ioctl(&ctx, arg1.into(), arg2 as _, arg3 as _).await,
        0x11 => {
            sys_pread64(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x12 => {
            sys_pwrite64(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x13 => sys_readv(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x14 => sys_writev(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x15 => {
            sys_preadv(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x16 => {
            sys_pwritev(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x28 => {
            sys_sendfile(
                &ctx,
                arg1.into(),
                arg2.into(),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x48 => sys_fcntl(&ctx, arg1.into(), arg2 as _, arg3 as _).await,
        0x4d => sys_ftruncate(&ctx, arg1.into(), arg3 as _).await,
        0x4f => sys_getcwd(&ctx, TUA::from_value(arg1 as _), arg2 as _).await,
        0x50 => sys_chdir(&ctx, TUA::from_value(arg1 as _)).await,
        0x51 => sys_fchdir(&ctx, arg1.into()).await,
        0x5b => sys_fchmod(&ctx, arg1.into(), arg3 as _).await,
        0x5d => sys_fchown(&ctx, arg1.into(), arg3 as _, arg4 as _).await,
        0x89 => sys_statfs(&ctx, TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        0x8a => sys_fstatfs(&ctx, arg1.into(), TUA::from_value(arg2 as _)).await,
        0xd9 => sys_getdents64(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x132 => sys_syncfs(&ctx, arg1.into()).await,

        // ── At-operations (openat=256, mkdirat=257, etc.) ────────
        0x100 => {
            sys_openat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x101 => sys_mkdirat(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x103 => {
            sys_fchownat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                arg5 as _,
            )
            .await
        }
        0x104 => {
            sys_newfstatat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x106 => sys_unlinkat(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x107 => {
            sys_renameat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3.into(),
                TUA::from_value(arg4 as _),
            )
            .await
        }
        0x108 => {
            sys_linkat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3.into(),
                TUA::from_value(arg4 as _),
                arg5 as _,
            )
            .await
        }
        0x109 => {
            sys_symlinkat(
                &ctx,
                TUA::from_value(arg1 as _),
                arg2.into(),
                TUA::from_value(arg3 as _),
            )
            .await
        }
        0x10a => {
            sys_readlinkat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x10b => {
            sys_fchmodat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x10c => sys_faccessat(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        0x10d => {
            sys_faccessat2(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0x118 => {
            sys_utimensat(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x13c => {
            sys_renameat2(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3.into(),
                TUA::from_value(arg4 as _),
                arg5 as _,
            )
            .await
        }
        0x14c => {
            sys_statx(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                TUA::from_value(arg5 as _),
            )
            .await
        }

        // ── Memory management ───────────────────────────────────
        0x09 => sys_mmap(&ctx, arg1, arg2, arg3, arg4, arg5.into(), arg6).await,
        0x0a => sys_mprotect(&ctx, VA::from_value(arg1 as _), arg2 as _, arg3 as _),
        0x0b => sys_munmap(&ctx, VA::from_value(arg1 as _), arg2 as _).await,
        0x0c => sys_brk(&ctx, VA::from_value(arg1 as _))
            .await
            .map_err(|e| match e {}),
        0x19 => {
            sys_mremap(
                &ctx,
                VA::from_value(arg1 as _),
                arg2 as _,
                arg3 as _,
                arg4,
                VA::from_value(arg5 as _),
            )
            .await
        }
        0x1b => sys_mincore(&ctx, arg1, arg2 as _, TUA::from_value(arg3 as _)).await,
        0x136 => {
            sys_process_vm_readv(
                arg1 as _,
                TUA::from_value(arg2 as _),
                arg3 as _,
                TUA::from_value(arg4 as _),
                arg5 as _,
                arg6 as _,
            )
            .await
        }

        // ── Scheduling ──────────────────────────────────────────
        0x18 => sys_sched_yield(),
        0xcb => sys_sched_setaffinity(&ctx, arg1 as _, arg2 as _, TUA::from_value(arg3 as _)),
        0xcc => sys_sched_getaffinity(&ctx, arg1 as _, arg2 as _, TUA::from_value(arg3 as _)),

        // ── Signal operations ───────────────────────────────────
        0x0d => {
            sys_rt_sigaction(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x0e => {
            sys_rt_sigprocmask(
                &mut ctx,
                arg1 as _,
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }
        0x83 => sys_sigaltstack(&ctx, TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        0x121 => {
            sys_signalfd4(
                &ctx,
                arg1 as _,
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
            )
            .await
        }
        0xc8 => sys_tkill(&ctx, arg1 as _, arg2.into()),

        // ── I/O multiplexing ────────────────────────────────────
        0x123 => sys_epoll_create1(&ctx, arg1 as _).await,
        0xe8 => {
            sys_epoll_ctl(
                &ctx,
                arg1.into(),
                arg2 as _,
                arg3.into(),
                TUA::from_value(arg4 as _),
            )
            .await
        }
        0x119 => {
            sys_epoll_pwait(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                TUA::from_value(arg5 as _),
                arg6 as _,
            )
            .await
        }
        0x10e => {
            sys_pselect6(
                &ctx,
                arg1 as _,
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                TUA::from_value(arg4 as _),
                TUA::from_value(arg5 as _),
                TUA::from_value(arg6 as _),
            )
            .await
        }
        0x10f => {
            sys_ppoll(
                &ctx,
                TUA::from_value(arg1 as _),
                arg2 as _,
                TUA::from_value(arg3 as _),
                TUA::from_value(arg4 as _),
                arg5 as _,
            )
            .await
        }

        // ── Inotify ─────────────────────────────────────────────
        0x126 => sys_inotify_init1(&ctx, arg1 as _).await,
        0xfd => {
            sys_inotify_add_watch(&ctx, arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await
        }
        0xfe => sys_inotify_rm_watch(&ctx, arg1.into(), arg2 as i32).await,

        // ── Process management ──────────────────────────────────
        0x38 => {
            sys_clone(
                &ctx,
                arg1 as _,
                UA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                TUA::from_value(arg5 as _),
                arg4 as _,
            )
            .await
        }
        0x3b => {
            sys_execve(
                &mut ctx,
                TUA::from_value(arg1 as _),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
            )
            .await
        }

        // ── Exit ────────────────────────────────────────────────
        0x3c => {
            let _ = sys_exit(&mut ctx, arg1 as _).await;
            debug_assert!(
                sched::current_work()
                    .state
                    .load(core::sync::atomic::Ordering::Acquire)
                    == TaskState::Finished
            );
            return;
        }
        0xe6 => {
            let _ = sys_exit_group(&ctx, arg1 as _).await;
            debug_assert!(
                sched::current_work()
                    .state
                    .load(core::sync::atomic::Ordering::Acquire)
                    == TaskState::Finished
            );
            return;
        }

        // ── Wait ────────────────────────────────────────────────
        0x3d => {
            sys_wait4(
                &ctx,
                arg1.cast_signed() as _,
                TUA::from_value(arg2 as _),
                arg3 as _,
                TUA::from_value(arg4 as _),
            )
            .await
        }
        0xf7 => {
            sys_waitid(
                &ctx,
                arg1 as _,
                arg2 as _,
                TUA::from_value(arg3 as _),
                arg4 as _,
                TUA::from_value(arg5 as _),
            )
            .await
        }

        // ── Process credentials ─────────────────────────────────
        0x27 => sys_getpid(&ctx).map_err(|e| match e {}),
        0x6e => sys_getppid(&ctx).map_err(|e| match e {}),
        0x66 => sys_getuid(&ctx).map_err(|e| match e {}),
        0x6b => sys_geteuid(&ctx).map_err(|e| match e {}),
        0x68 => sys_getgid(&ctx).map_err(|e| match e {}),
        0x6c => sys_getegid(&ctx).map_err(|e| match e {}),
        0xba => sys_gettid(&ctx).map_err(|e| match e {}),
        0x79 => sys_getpgid(&ctx, arg1 as _),
        0x6d => sys_setpgid(&ctx, arg1 as _, Pgid(arg2 as _)),
        0x3e => sys_kill(&ctx, arg1 as _, arg2.into()),
        0x69 => sys_setuid(&ctx, arg1 as _),
        0x6a => sys_setgid(&ctx, arg1 as _),
        0x71 => sys_setreuid(&ctx, arg1 as _, arg2 as _),
        0x72 => sys_setregid(&ctx, arg1 as _, arg2 as _),
        0x75 => sys_setresuid(&ctx, arg1 as _, arg2 as _, arg3 as _),
        0x77 => sys_setresgid(&ctx, arg1 as _, arg2 as _, arg3 as _),
        0x7a => sys_setfsuid(&ctx, arg1 as _).map_err(|e| match e {}),
        0x7b => sys_setfsgid(&ctx, arg1 as _).map_err(|e| match e {}),
        0x95 => sys_umask(&ctx, arg1 as _).map_err(|e| match e {}),

        // ── Capabilities ────────────────────────────────────────
        0x7d => sys_capget(&ctx, TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        0x7e => sys_capset(&ctx, TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,

        // ── Ptrace ──────────────────────────────────────────────
        0x65 => {
            sys_ptrace(
                &ctx,
                arg1 as _,
                arg2 as _,
                TUA::from_value(arg3 as _),
                TUA::from_value(arg4 as _),
            )
            .await
        }

        // ── Time ────────────────────────────────────────────────
        0x60 => sys_gettimeofday(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        0xe2 => sys_clock_settime(arg1 as _, TUA::from_value(arg2 as _)).await,
        0xe3 => sys_clock_gettime(&ctx, arg1 as _, TUA::from_value(arg2 as _)).await,
        0xe5 => {
            sys_clock_nanosleep(
                arg1 as _,
                arg2 as _,
                TUA::from_value(arg3 as _),
                TUA::from_value(arg4 as _),
            )
            .await
        }
        0x23 => sys_nanosleep(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        0x24 => sys_getitimer(&ctx, arg1 as _, TUA::from_value(arg2 as _)).await,
        0x26 => {
            sys_setitimer(
                &ctx,
                arg1 as _,
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
            )
            .await
        }

        // ── Identity / hostname / info ──────────────────────────
        0x3f => sys_uname(TUA::from_value(arg1 as _)).await,
        0xaa => sys_sethostname(&ctx, TUA::from_value(arg1 as _), arg2 as _).await,
        0x63 => sys_sysinfo(TUA::from_value(arg1 as _)).await,

        // ── Resource limits ─────────────────────────────────────
        0x12e => {
            sys_prlimit64(
                &ctx,
                arg1 as _,
                arg2 as _,
                TUA::from_value(arg3 as _),
                TUA::from_value(arg4 as _),
            )
            .await
        }

        // ── Prctl ───────────────────────────────────────────────
        0x9d => sys_prctl(&ctx, arg1 as _, arg2, arg3).await,

        // ── Reboot ──────────────────────────────────────────────
        0xa9 => sys_reboot(&ctx, arg1 as _, arg2 as _, arg3 as _, arg4 as _).await,

        // ── Network ─────────────────────────────────────────────
        0x29 => sys_socket(&ctx, arg1 as _, arg2 as _, arg3 as _).await,
        0x2a => sys_connect(&ctx, arg1.into(), UA::from_value(arg2 as _), arg3 as _).await,
        0x2b => {
            sys_accept(
                &ctx,
                arg1.into(),
                UA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
            )
            .await
        }
        0x2c => {
            sys_sendto(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                UA::from_value(arg5 as _),
                arg6 as _,
            )
            .await
        }
        0x2d => {
            sys_recvfrom(
                &ctx,
                arg1.into(),
                UA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                UA::from_value(arg5 as _),
                TUA::from_value(arg6 as _),
            )
            .await
        }
        0x30 => sys_shutdown(&ctx, arg1.into(), arg2 as _).await,
        0x31 => sys_bind(&ctx, arg1.into(), UA::from_value(arg2 as _), arg3 as _).await,
        0x32 => sys_listen(&ctx, arg1.into(), arg2 as _).await,
        0x120 => {
            sys_accept4(
                &ctx,
                arg1.into(),
                UA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
            )
            .await
        }

        // ── Duplicate / pipe ────────────────────────────────────
        0x20 => sys_dup(&ctx, arg1.into()),
        0x124 => sys_dup3(&ctx, arg1.into(), arg2.into(), arg3 as _),
        0x125 => sys_pipe2(&ctx, TUA::from_value(arg1 as _), arg2 as _).await,

        // ── Random / memfd ──────────────────────────────────────
        0x13e => sys_getrandom(TUA::from_value(arg1 as _), arg2 as _, arg3 as _).await,
        0x13f => sys_memfd_create(&ctx, TUA::from_value(arg1 as _), arg2 as _).await,

        // ── copy_file_range / preadv2 / pwritev2 ────────────────
        0x146 => {
            sys_copy_file_range(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3.into(),
                TUA::from_value(arg4 as _),
                arg5 as _,
                arg6 as _,
            )
            .await
        }
        0x147 => {
            sys_preadv2(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                arg5 as _,
            )
            .await
        }
        0x148 => {
            sys_pwritev2(
                &ctx,
                arg1.into(),
                TUA::from_value(arg2 as _),
                arg3 as _,
                arg4 as _,
                arg5 as _,
            )
            .await
        }

        // ── close_range / pidfd ─────────────────────────────────
        0x162 => sys_close_range(&ctx, arg1.into(), arg2.into(), arg3 as _).await,
        0x16c => sys_pidfd_open(&ctx, arg1 as _, arg2 as _).await,

        // ── mount / chroot ──────────────────────────────────────
        0xa5 => {
            sys_mount(
                &ctx,
                TUA::from_value(arg1 as _),
                TUA::from_value(arg2 as _),
                TUA::from_value(arg3 as _),
                arg4 as _,
                TUA::from_value(arg5 as _),
            )
            .await
        }
        0xa1 => sys_chroot(&ctx, TUA::from_value(arg1 as _)).await,

        // ── futex / robust_list / tid_address ───────────────────
        0xca => {
            sys_futex(
                &ctx,
                TUA::from_value(arg1 as _),
                arg2 as _,
                arg3 as _,
                TUA::from_value(arg4 as _),
                TUA::from_value(arg5 as _),
                arg6 as _,
            )
            .await
        }
        0x111 => sys_set_robust_list(&mut ctx, TUA::from_value(arg1 as _), arg2 as _).await,
        0xda => sys_set_tid_address(&mut ctx, TUA::from_value(arg1 as _)),

        _ => {
            panic!(
                "Unhandled syscall 0x{nr:x}, PC: 0x{:x}",
                ctx.task().ctx.user().rip
            );
        }
    };

    let ret_val = match res {
        Ok(v) => v as isize,
        Err(e) => kern_err_to_syscall(e),
    };

    ctx.task_mut().ctx.user_mut().rax = ret_val.cast_unsigned() as u64;
    ptrace_stop(&ctx, TracePoint::SyscallExit).await;
    ctx.task_mut().update_accounting(None);
    ctx.task_mut().in_syscall = false;
}
