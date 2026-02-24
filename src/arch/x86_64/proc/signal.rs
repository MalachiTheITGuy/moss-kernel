use libkernel::error::Result;
use crate::process::thread_group::signal::{SigId, ksigaction::UserspaceSigAction};
use crate::arch::x86_64::exceptions::ExceptionState;

pub async fn do_signal(
    _sig: SigId,
    _action: UserspaceSigAction,
) -> Result<ExceptionState> {
    Err(libkernel::error::KernelError::NotSupported)
}

pub async fn do_signal_return() -> Result<ExceptionState> {
    Err(libkernel::error::KernelError::NotSupported)
}
