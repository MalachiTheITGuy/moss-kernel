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
            select::{sys_ppoll, sys_pselect6},
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
    },
    sched::{current::current_task, sys_sched_yield, uspc_ret::dispatch_userspace_task},
    spawn_kernel_work,
};
use alloc::boxed::Box;
use libkernel::{
    error::{KernelError, syscall_error::kern_err_to_syscall},
    memory::address::{TUA, UA, VA},
};
use super::ExceptionState;

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
        2 => sys_openat(0x9c.into(), TUA::from_value(arg1 as _), arg2 as _, arg3 as _).await,
        3 => sys_close(arg1.into()).await,
        4 | 6 => sys_newfstatat(0x9c.into(), TUA::from_value(arg1 as _), TUA::from_value(arg2 as _), arg3 as _).await,
        5 => sys_fstat(arg1.into(), TUA::from_value(arg2 as _)).await,
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
        21 => sys_faccessat(0x9c.into(), TUA::from_value(arg1 as _), arg2 as _).await,
        24 => sys_sched_yield(),
        32 => sys_dup(arg1.into()),
        33 => sys_dup3(arg1.into(), arg2.into(), 0),
        35 => sys_nanosleep(TUA::from_value(arg1 as _), TUA::from_value(arg2 as _)).await,
        39 => sys_getpid().map_err(|e| match e {}),
        56 => sys_clone(arg1 as _, UA::from_value(arg2 as _), TUA::from_value(arg3 as _), TUA::from_value(arg5 as _), arg4 as _).await,
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
        83 => sys_mkdirat(0x9c.into(), TUA::from_value(arg1 as _), arg2 as _).await,
        158 => sys_arch_prctl(arg1 as _, arg2).await,
        186 => sys_gettid().map_err(|e| match e {}),
        202 => sys_futex(TUA::from_value(arg1 as _), arg2 as _, arg3 as _, TUA::from_value(arg4 as _), TUA::from_value(arg5 as _), arg6 as _).await,
        218 => sys_set_tid_address(TUA::from_value(arg1 as _)),
        231 => {
            let _ = sys_exit_group(arg1 as _).await;
            return;
        }
        257 => sys_openat(arg1.into(), TUA::from_value(arg2 as _), arg3 as _, arg4 as _).await,
        _ => panic!("Unhandled x86_64 syscall {nr}, RIP: 0x{:x}", current_task().ctx.user().rip),
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

async fn sys_arch_prctl(code: i32, _addr: u64) -> libkernel::error::Result<usize> {
    const ARCH_SET_GS: i32 = 0x1001;
    const ARCH_SET_FS: i32 = 0x1002;
    match code {
        ARCH_SET_FS | ARCH_SET_GS => {
            // TODO: Handle FS_BASE/GS_BASE in Task state and apply on context switch.
            // Until implemented, report that this operation is not supported.
            Err(KernelError::NotSupported)
        }
        _ => Err(KernelError::NotSupported),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn x86_64_syscall_handler(state: *mut ExceptionState) -> *mut ExceptionState {
    let mut task = current_task();
    task.ctx.save_user_ctx(state);
    
    spawn_kernel_work(handle_syscall());
    
    dispatch_userspace_task(state);
    
    state
}
