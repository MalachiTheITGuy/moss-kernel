//! x86_64 process management.

use alloc::sync::Arc;
use crate::process::Task;

pub mod idle;
pub mod signal;
pub mod vdso;

pub fn context_switch(new: Arc<Task>) {
    // TODO: implement x86_64 context switch (load CR3, switch stacks, etc.)
    todo!("x86_64 context_switch")
}
