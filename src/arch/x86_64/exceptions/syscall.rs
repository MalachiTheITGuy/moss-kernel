use libkernel::error::{syscall_error::kern_err_to_syscall, KernelError};

#[no_mangle]
pub extern "C" fn x86_64_syscall_handler(
    nr: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> i64 {
    let result = match nr {
        0 => sys_read(arg1 as i32, arg2 as *mut u8, arg3 as usize),
        1 => sys_write(arg1 as i32, arg2 as *const u8, arg3 as usize),
        3 => sys_close(arg1 as i32),
        8 => sys_lseek(arg1 as i32, arg2 as i64, arg3 as u32),
        12 => sys_brk(arg1 as usize),
        13 => Ok(0),
        14 => Ok(0),
        24 => sys_sched_yield(),
        35 => sys_nanosleep(),
        39 => sys_getpid(),
        60 => sys_exit(arg1 as i32),
        63 => Err(KernelError::NotSupported),
        80 => Err(KernelError::NotSupported),
        96 => sys_umask(arg1 as u16),
        102 => sys_getuid(),
        104 => sys_getgid(),
        107 => sys_geteuid(),
        108 => sys_getegid(),
        110 => sys_getppid(),
        186 => sys_gettid(),
        218 => sys_set_tid_address(),
        231 => sys_exit_group(arg1 as i32),
        _ => Err(KernelError::NotSupported),
    };

    match result {
        Ok(v) => v as i64,
        Err(e) => kern_err_to_syscall(e) as i64,
    }
}

fn sys_read(_fd: i32, _buf: *mut u8, _count: usize) -> Result<i64, KernelError> {
    Err(KernelError::BadFd)
}

fn sys_write(fd: i32, buf: *const u8, count: usize) -> Result<i64, KernelError> {
    if fd == 1 || fd == 2 {
        unsafe {
            let slice = core::slice::from_raw_parts(buf, count);
            for &byte in slice {
                let _ = core::ptr::write_volatile(0x3F8 as *mut u8, byte);
            }
        }
        return Ok(count as i64);
    }
    Err(KernelError::BadFd)
}

fn sys_close(_fd: i32) -> Result<i64, KernelError> {
    Err(KernelError::BadFd)
}

fn sys_lseek(_fd: i32, _offset: i64, _whence: u32) -> Result<i64, KernelError> {
    Err(KernelError::BadFd)
}

fn sys_brk(_addr: usize) -> Result<i64, KernelError> {
    Err(KernelError::NotSupported)
}

fn sys_sched_yield() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_nanosleep() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_exit(_status: i32) -> Result<i64, KernelError> {
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

fn sys_exit_group(status: i32) -> Result<i64, KernelError> {
    sys_exit(status)
}

fn sys_getpid() -> Result<i64, KernelError> {
    Ok(1)
}

fn sys_getppid() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_getuid() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_geteuid() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_getgid() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_getegid() -> Result<i64, KernelError> {
    Ok(0)
}

fn sys_gettid() -> Result<i64, KernelError> {
    Ok(1)
}

fn sys_set_tid_address() -> Result<i64, KernelError> {
    Ok(1)
}

fn sys_umask(mode: u16) -> Result<i64, KernelError> {
    Ok((0o022 & mode) as i64)
}
