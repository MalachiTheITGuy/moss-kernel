use core::fmt::Display;

use alloc::{boxed::Box, sync::Arc};
use libkernel::error::Result;

use super::{Driver, DriverManager};

#[cfg(target_arch = "aarch64")]
bitflags::bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct FdtFlags: u32 {
        const ACTIVE_CONSOLE = 1;
    }
}

#[cfg(target_arch = "aarch64")]
#[repr(C)]
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DeviceMatchType {
    FdtCompatible(&'static str),
}

#[derive(Clone)]
pub enum DeviceDescriptor {
    #[cfg(target_arch = "aarch64")]
    Fdt(fdt_parser::Node<'static>, FdtFlags),
}

impl Display for DeviceDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(target_arch = "aarch64")]
            DeviceDescriptor::Fdt(node, _) => f.write_str(node.name),
            #[cfg(not(target_arch = "aarch64"))]
            _ => unreachable!("no DeviceDescriptor variants on x86_64"),
        }
    }
}

pub type ProbeFn =
    Box<dyn Fn(&mut DriverManager, DeviceDescriptor) -> Result<Arc<dyn Driver>> + Send>;
