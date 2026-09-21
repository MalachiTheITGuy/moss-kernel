#[cfg(target_arch = "aarch64")]
pub mod arm_gic_v2;
#[cfg(target_arch = "aarch64")]
pub mod arm_gic_v3;

#[cfg(target_arch = "x86_64")]
pub mod x86_64_ioapic;
#[cfg(target_arch = "x86_64")]
pub mod x86_64_lapic;
