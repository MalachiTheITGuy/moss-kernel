#[allow(unused_imports)]
use crate::{
    arch::{Arch, ArchImpl},
    clock::{
        gettime::sys_clock_gettime,
        settime::sys_clock_settime,
        timeofday::{sys_gettimeofday, sys_settimeofday},
    },
    fs::{
        dir::sys_getdents64,
        pipe::sys_pipe2,
        syscalls::{
            at::{
                access::{sys_faccessat, sys_faccessat2},
                chmod::sys_fchmodat,
                chown::sys_fchownat,
                handle::sys_name_to_handle_at,
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
            getxattr::{sys_fgetxattr, sys_getxattr, sys_lgetxattr},
            ioctl::sys_ioctl,
            iov::{sys_preadv, sys_preadv2, sys_pwritev, sys_pwritev2, sys_readv, sys_writev},
            listxattr::{sys_flistxattr, sys_listxattr, sys_llistxattr},
            removexattr::{sys_fremovexattr, sys_lremovexattr, sys_removexattr},
            rw::{sys_pread64, sys_pwrite64, sys_read, sys_write},
            seek::sys_lseek,
            setxattr::{sys_fsetxattr, sys_lsetxattr, sys_setxattr},
            splice::sys_sendfile,
            stat::sys_fstat,
            statfs::{sys_fstatfs, sys_statfs},
            sync::{sys_fdatasync, sys_fsync, sys_sync, sys_syncfs},
            trunc::{sys_ftruncate, sys_truncate},
        },
    },
    kernel::{
        hostname::sys_sethostname, power::sys_reboot, rand::sys_getrandom, sysinfo::sys_sysinfo,
        uname::sys_uname,
    },
    memory::{
        brk::sys_brk,
        mincore::sys_mincore,
        mmap::{sys_mmap, sys_mprotect, sys_munmap},
        process_vm::sys_process_vm_readv,
    },
    process::{
        TaskState,
        caps::{sys_capget, sys_capset},
        clone::sys_clone,
        creds::{
            sys_getegid, sys_geteuid, sys_getgid, sys_getresgid, sys_getresuid, sys_getsid,
            sys_gettid, sys_getuid, sys_setfsgid, sys_setfsuid, sys_setsid,
        },
        exec::sys_execve,
        exit::{sys_exit, sys_exit_group},
        fd_table::{
            dup::{sys_dup, sys_dup3},
            fcntl::sys_fcntl,
            select::{sys_poll, sys_ppoll, sys_pselect6},
        },
        prctl::sys_prctl,
        ptrace::{TracePoint, ptrace_stop, sys_ptrace},
        sleep::{sys_clock_nanosleep, sys_nanosleep},
        thread_group::{
            Pgid,
            pid::{sys_getpgid, sys_getpid, sys_getppid, sys_setpgid},
            rsrc_lim::sys_prlimit64,
            signal::{
                SigId,
                kill::sys_kill,
                sigaction::sys_rt_sigaction,
                sigaltstack::sys_sigaltstack,
                sigprocmask::sys_rt_sigprocmask,
            },
            umask::sys_umask,
            wait::{sys_wait4, sys_waitid},
        },
        threading::{futex::sys_futex, sys_set_robust_list, sys_set_tid_address},
        creds::sys_unshare,
    },
    sched::{current::current_task, sys_sched_yield, uspc_ret::dispatch_userspace_task},
    spawn_kernel_work,
};
use alloc::boxed::Box;
use libkernel::{
    error::{KernelError, syscall_error::kern_err_to_syscall},
    memory::address::{TUA, UA, VA},
};
use crate::process::fd_table::{AT_FDCWD, Fd};
use super::{ExceptionState, read_fs_base, write_fs_base};

pub async fn handle_syscall() {
    current_task().update_accounting(None);
    current_task().in_syscall = true;
    ptrace_stop(TracePoint::SyscallEntry).await;

    let (nr, arg1, arg2, arg3, arg4, arg5, arg6) = {
        let task = current_task();
        let state = task.ctx.user();

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

    let res = match nr {
        0 => sys_read(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        1 => sys_write(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        2 => sys_openat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), arg2 as _, arg3 as _).await,
        3 => sys_close(arg1.into()).await,
        4 | 6 => sys_newfstatat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), arg3 as _).await,
        5 => sys_fstat(arg1.into(), TUA::from_value(arg2 as _)).await,
        7 => sys_poll(TUA::from_value(arg1 as _), arg2 as _, arg3 as i32).await,
        8 => sys_lseek(arg1.into(), arg2 as _, arg3 as _).await,
        9 => sys_mmap(arg1, arg2, arg3, arg4, arg5.into(), arg6).await,
        10 => sys_mprotect(VA::from_value(arg1 as _), arg2 as _, arg3 as _),
        11 => sys_munmap(VA::from_value(arg1 as usize), arg2 as _).await,
        12 => sys_brk(VA::from_value(arg1 as _)).await.map_err(|e| match e {}),
        13 => sys_rt_sigaction(arg1.into(), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), arg4 as _).await,
        14 => sys_rt_sigprocmask(arg1 as _, TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), arg4 as _).await,
        15 => {
            current_task().ctx.put_signal_work(Box::pin(ArchImpl::do_signal_return()));
            return;
        }
        16 => sys_ioctl(arg1.into(), arg2 as _, arg3 as _).await,
        19 => sys_readv(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        20 => sys_writev(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        21 => sys_faccessat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), arg2 as _).await,
        24 => sys_sched_yield(),
        32 => sys_dup(arg1.into()),
        33 => sys_dup3(arg1.into(), arg2.into(), 0),
        35 => sys_nanosleep(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        39 => sys_getpid().map_err(|e| match e {}),
        56 => sys_clone(arg1 as _, UA::from_value(arg2 as _), TUA::from_value(arg3 as _), TUA::from_value(arg5 as _), arg4 as _).await,
        57 => sys_clone(17, UA::null(), TUA::null(), TUA::null(), 0).await, // fork = clone(SIGCHLD)
        59 => sys_execve(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _)).await,
        60 => {
            let _ = sys_exit(arg1 as _).await;
            return;
        }
        61 => sys_wait4(arg1.cast_signed() as _, TUA::from_value(arg2 as _), arg3 as _, TUA::from_value(arg4 as _)).await,
        62 => sys_kill(arg1 as _, arg2.into()),
        63 => sys_uname(TUA::from_value(arg1 as _)).await,
        72 => sys_fcntl(arg1.into(), arg2 as _, arg3 as _).await,
        77 => sys_ftruncate(arg1.into(), arg2 as _).await,
        78 => sys_getdents64(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        79 => sys_getcwd(TUA::from_value(arg1 as _), arg2 as _).await,
        80 => sys_chdir(TUA::from_value(arg1 as _)).await,
        83 => sys_mkdirat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), arg2 as _).await,
        84 => sys_unlinkat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), 0x200).await, // rmdir via unlinkat(AT_FDCWD, path, AT_REMOVEDIR)
        85 => sys_openat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), 0x241, arg2 as _).await, // creat = open(O_CREAT|O_WRONLY|O_TRUNC)
        86 => sys_linkat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), Fd(AT_FDCWD), TUA::from_value(arg2 as _), 0).await,
        87 => sys_unlinkat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), 0).await,
        88 => sys_symlinkat(TUA::from_value(arg1 as _), Fd(AT_FDCWD), TUA::from_value(arg2 as _)).await,
        89 => sys_readlinkat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), UA::from_value(arg2 as _), arg3 as _).await,
        90 => sys_fchmodat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), arg2 as _, 0).await,
        91 => sys_fchown(arg1.into(), arg2 as _, arg3 as _).await,
        92 => sys_fchownat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), arg2 as _, arg3 as _, 0).await,
        93 => sys_fchdir(arg1.into()).await,
        95 => sys_umask(arg1 as _).map_err(|e| match e {}),
        96 => sys_gettimeofday(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        102 => sys_getuid().map_err(|e| match e {}),

        107 => sys_geteuid().map_err(|e| match e {}),
        108 => sys_getgid().map_err(|e| match e {}),
        109 => sys_setpgid(arg1 as _, Pgid(arg2 as u32)),
        110 => sys_getppid().map_err(|e| match e {}),
        111 => sys_getsid(arg1 as _),
        112 => sys_setpgid(arg1 as _, Pgid(arg2 as u32)),
        113 => sys_getpgid(arg1 as _),
        121 => sys_unshare(arg1 as _),
        131 => sys_sigaltstack(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        132 => sys_utimensat(Fd(AT_FDCWD), TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), 0).await, // utime -> utimensat
        157 => sys_prctl(arg1 as _, arg2 as _, arg3 as _).await,
        158 => sys_arch_prctl(arg1 as _, arg2).await,
        160 => sys_settimeofday(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        162 => sys_sync().await,
        165 => sys_getresuid(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _)).await,
        168 => sys_getresgid(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _)).await,
        186 => sys_gettid().map_err(|e| match e {}),
        197 => sys_fstat(arg1.into(), TUA::from_value(arg2 as _)).await,
        200 => Err(libkernel::error::KernelError::NotSupported), // sys_setns - namespaces not implemented
        202 => sys_futex(TUA::from_value(arg1 as _), arg2 as _, arg3 as _, TUA::from_value(arg4 as _), TUA::from_value(arg5 as _), arg6 as _).await,

        217 => sys_getdents64(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        218 => sys_set_tid_address(TUA::from_value(arg1 as _)),
        228 => sys_clock_gettime(arg1 as _, TUA::from_value(arg2 as _)).await,
        230 => sys_clock_nanosleep(arg1 as _, arg2 as _, TUA::from_value(arg3 as _), TUA::from_value(arg4 as _)).await,
        231 => {
            let _ = sys_exit_group(arg1 as _).await;
            return;
        }

        257 => sys_openat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, arg4 as _).await,
        258 => sys_mkdirat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        261 => sys_utimensat(arg1.into(), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), arg4 as _).await,
        262 => sys_newfstatat(arg1.into(), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), arg4 as _).await,
        263 => sys_unlinkat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _).await,
        265 => sys_linkat(arg1.into(), TUA::from_value(arg2 as _), arg3.into(), TUA::from_value(arg4 as _), arg5 as _).await,
        266 => sys_symlinkat(TUA::from_value(arg1 as _), arg2.into(), TUA::from_value(arg3 as _)).await,
        267 => sys_readlinkat(arg1.into(), TUA::from_value(arg2 as _), UA::from_value(arg3 as _), arg4 as _).await,
        268 => sys_fchmodat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, 0).await,
        269 => sys_faccessat(arg1.into(), TUA::from_value(arg2 as _), arg2 as _).await,
        270 => sys_pselect6(arg1 as _, TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), TUA::from_value(arg4 as _), TUA::from_value(arg5 as _), TUA::from_value(arg6 as _)).await,
        271 => sys_ppoll(TUA::from_value(arg1 as _), arg2 as _, TUA::from_value(arg3 as _), TUA::from_value(arg4 as _), arg5 as _).await,
        280 => sys_utimensat(arg1.into(), TUA::from_value(arg2 as _), TUA::from_value(arg3 as _), arg4 as _).await,

        292 => sys_dup3(arg1.into(), arg2.into(), arg3 as _),
        293 => sys_pipe2(TUA::from_value(arg1 as _), arg2 as _).await,
        295 => sys_openat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, arg4 as _).await,
        302 => sys_prlimit64(arg1 as _, arg2 as _, TUA::from_value(arg3 as _), TUA::from_value(arg4 as _)).await,
        306 => sys_syncfs(arg1.into()).await,
        316 => sys_renameat2(arg1.into(), TUA::from_value(arg2 as _), arg3.into(), TUA::from_value(arg4 as _), arg5 as _).await,
        318 => sys_getrandom(TUA::from_value(arg1 as _), arg2 as _, arg3 as _).await,
        322 => sys_statx(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, arg4 as _, TUA::from_value(arg5 as _)).await,
        334 => sys_faccessat2(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, arg4 as _).await,
        _ => {
            log::warn!("Unhandled x86_64 syscall {nr}, returning ENOSYS. RIP: 0x{:x}", current_task().ctx.user().rip);
            Err(libkernel::error::KernelError::NotSupported)
        }
    };

    let ret_val = match res {
        Ok(v) => v as isize,
        Err(e) => kern_err_to_syscall(e),
    };

    current_task().ctx.user_mut().rax = ret_val.cast_unsigned() as u64;
    ptrace_stop(TracePoint::SyscallExit).await;
    current_task().update_accounting(None);
    current_task().in_syscall = false;
}

async fn sys_arch_prctl(code: i32, addr: u64) -> libkernel::error::Result<usize> {
    const ARCH_SET_GS: i32 = 0x1001;
    const ARCH_SET_FS: i32 = 0x1002;
    const ARCH_GET_FS: i32 = 0x1003;
    const ARCH_GET_GS: i32 = 0x1004;
    match code {
        ARCH_SET_FS => {
            // Set the FS base (used for TLS in the musl ABI).
            // Store it in the task context; write_fs_base() applies it on
            // return to userspace.
            ArchImpl::set_user_thread_area(current_task().ctx.user_mut(), VA::from_value(addr as usize));
            Ok(0)
        }
        ARCH_SET_GS => {
            // GS_BASE is used for per-CPU data in kernels, but some user-space
            // runtimes also use it.  Store in gs_base (not yet tracked
            // separately, so write the MSR directly for now).
            unsafe {
                core::arch::asm!(
                    "wrmsr",
                    in("ecx") 0xC0000101u32, // IA32_GS_BASE (user GS)
                    in("eax") (addr & 0xFFFF_FFFF) as u32,
                    in("edx") (addr >> 32) as u32,
                )
            }
            Ok(0)
        }
        ARCH_GET_FS => {
            // Return the current FS base stored in the task context.
            Ok(current_task().ctx.user().fs_base as usize)
        }
        ARCH_GET_GS => {
            // Return the current GS base from the MSR.
            let lo: u32;
            let hi: u32;
            unsafe {
                core::arch::asm!(
                    "rdmsr",
                    in("ecx") 0xC0000101u32,
                    out("eax") lo,
                    out("edx") hi,
                )
            }
            Ok((hi as usize) << 32 | lo as usize)
        }
        _ => Err(libkernel::error::KernelError::NotSupported),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn x86_64_syscall_handler(state: *mut ExceptionState) -> *mut ExceptionState {
    // Syscalls always come from user space.  Save the current FS_BASE MSR into
    // the exception frame (the assembly stub pushed a $0 placeholder).
    unsafe { (*state).fs_base = read_fs_base() };

    {
        let mut task = current_task();
        task.ctx.save_user_ctx(state);
    } // drop the borrow before handle_syscall() re-acquires it

    spawn_kernel_work(handle_syscall());
    
    dispatch_userspace_task(state);

    // Restore FS_BASE for whichever task dispatch_userspace_task chose to run.
    // restore_user_ctx has already written its ExceptionState (including fs_base)
    // into the stack frame; we apply that value to the MSR now.
    let new_fs_base = unsafe { (*state).fs_base };
    write_fs_base(new_fs_base);

    // If we're returning to a kernel thread, we should use the kernel CS.
    // Otherwise, we use the user CS.  dispatch_userspace_task will have
    // updated the state in place.
    let is_user = unsafe { ((*state).cs & 0x3) != 0 };
    if !is_user {
        unsafe { (*state).cs = crate::arch::x86_64::KERNEL_CS };
    }

    state
}
