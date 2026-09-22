//! x86_64 signal frame setup and delivery.
//!
//! Handles building the `RtSigFrame` on the user stack, setting up the
//! CPU registers for signal handler entry, and restoring context on
//! signal return.  The layout and flow mirrors the ARM64 implementation
//! in `arch/arm64/proc/signal.rs`.

use super::super::ptrace::X86_64PtraceGPRegs;
use super::vdso::VDSO_BASE;
use crate::{
    memory::uaccess::{UserCopyable, copy_from_user, copy_to_user},
    process::thread_group::signal::{
        SigId, ksigaction::UserspaceSigAction, sigaction::SigActionFlags,
    },
    sched::syscall_ctx::ProcessCtx,
};
use libkernel::{
    error::Result,
    memory::{
        PAGE_SIZE,
        address::{TUA, UA},
    },
};

/// The signal frame copied onto the user stack during signal delivery.
///
/// Mirrors the ARM64 `RtSigFrame` in `arch/arm64/proc/signal.rs`, but
/// uses `X86_64PtraceGPRegs` as the saved user context instead of
/// `ExceptionState`.
#[repr(C)]
#[derive(Clone, Copy)]
struct RtSigFrame {
    /// Saved user registers, restored on `sigreturn`.
    uctx: X86_64PtraceGPRegs,
    /// Previous alternate stack pointer, saved when `SA_ONSTACK` was used.
    /// `UA::null()` if no alternate stack was active.
    alt_stack_prev_addr: UA,
}

// SAFETY: The signal frame that's copied to user-space only contains
// information regarding this task's context and is made up of PoDs.
unsafe impl UserCopyable for RtSigFrame {}

/// Build a signal frame on the user stack and set up registers for
/// signal handler entry.
///
/// # Signal delivery ABI (x86_64)
///
/// The kernel prepares the following state before transferring control
/// to the signal handler:
///
/// ```text
///                ┌─────────────────────┐  ← new rsp
///                │   RtSigFrame        │
///                │   ├─ uctx (GPregs)  │
///                │   └─ alt_stack_prev │
///                └─────────────────────┘
///  rsp ─────────►                       rsp = addr (frame on stack)
///  rip ─────────► sa.action             handler entry point
///  rcx ─────────► restorer             return address (ret lands here)
///  rdi ─────────► signal number         first argument
/// ```
///
/// # Arguments
///
/// * `ctx` – Process context for the current task.
/// * `id` – The signal being delivered.
/// * `sa` – The userspace signal action (handler, flags, restorer, mask).
///
/// # Returns
///
/// The modified register state that will be restored to userspace.
pub async fn do_signal(
    ctx: ProcessCtx,
    id: SigId,
    sa: UserspaceSigAction,
) -> Result<X86_64PtraceGPRegs> {
    let task = ctx.task();
    let mut signal = task.process.signals.lock_save_irq();

    // Snapshot the current userspace register state.
    let saved_state = *task.ctx.user();
    let mut new_state = saved_state;
    let mut frame = RtSigFrame {
        uctx: saved_state,
        alt_stack_prev_addr: UA::null(),
    };

    // Use the provided restorer trampoline, or the one provided by the
    // VDSO if not.
    let restorer = sa
        .restorer
        .map(|x| x.value())
        .unwrap_or_else(|| VDSO_BASE.value());

    // Determine the frame address.  If SA_ONSTACK is set and the task has
    // an alternate signal stack, allocate the frame there.  Otherwise,
    // place it below the current stack pointer, aligned to PAGE_SIZE.
    let addr: TUA<RtSigFrame> = if sa.flags.contains(SigActionFlags::SA_ONSTACK)
        && let Some(alt_stack) = signal.alt_stack.as_mut()
        && let Some(alloc) = alt_stack.alloc_alt_stack::<RtSigFrame>()
    {
        frame.alt_stack_prev_addr = alloc.old_ptr;
        alloc.data_ptr.cast()
    } else {
        TUA::from_value(new_state.rsp as _)
            .sub_objs(1)
            .align(PAGE_SIZE)
    };

    drop(signal);

    copy_to_user(addr, frame).await?;

    // Set up registers for signal handler entry.
    //
    // x86_64 calling convention:
    //   rsp  = stack pointer (points at the RtSigFrame)
    //   rip  = signal handler entry point
    //   rcx  = return address (restorer trampoline or VDSO)
    //   rdi  = first argument (signal number)
    new_state.rsp = addr.value() as _;
    new_state.rip = sa.action.value() as _;
    new_state.rcx = restorer as _;
    new_state.rdi = id.user_id();

    Ok(new_state)
}

/// Restore the user context from a signal frame on the stack.
///
/// Called when the signal handler executes the `sigreturn` syscall
/// (or VDSO equivalent).  Reads the `RtSigFrame` from the user stack,
/// restores the alternate signal stack if one was saved, and returns
/// the original register state.
///
/// # Arguments
///
/// * `ctx` – Process context for the current task.
///
/// # Returns
///
/// The original register state saved in the signal frame.
pub async fn do_signal_return(ctx: ProcessCtx) -> Result<X86_64PtraceGPRegs> {
    let task = ctx.task();

    // The signal frame is at the current stack pointer.
    let sig_frame_addr: TUA<RtSigFrame> = TUA::from_value(task.ctx.user().rsp as _);

    let sig_frame = copy_from_user(sig_frame_addr).await?;

    // Restore the alternate signal stack if one was saved.
    if !sig_frame.alt_stack_prev_addr.is_null() {
        task.process
            .signals
            .lock_save_irq()
            .alt_stack
            .as_mut()
            .expect("Alt stack disappeared during use")
            .restore_alt_stack(sig_frame.alt_stack_prev_addr);
    }

    Ok(sig_frame.uctx)
}
