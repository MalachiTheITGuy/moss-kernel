use crate::arch::{Arch, ArchImpl};
use crate::process::thread_group::pid::PidT;
use crate::sched::syscall_ctx::ProcessCtx;
use crate::sched::schedule;

pub fn sys_sched_yield() -> libkernel::error::Result<usize> {
    schedule();
    Ok(0)
}

pub fn sys_sched_getaffinity(
    _ctx: &ProcessCtx,
    _pid: PidT,
    _size: usize,
    _mask: libkernel::memory::address::UA,
) -> libkernel::error::Result<usize> {
    Err(libkernel::error::KernelError::Other("sched_getaffinity not implemented"))
}

pub fn sys_sched_setaffinity(
    _ctx: &ProcessCtx,
    _pid: PidT,
    _size: usize,
    _mask: libkernel::memory::address::UA,
) -> libkernel::error::Result<usize> {
    Err(libkernel::error::KernelError::Other("sched_setaffinity not implemented"))
}
