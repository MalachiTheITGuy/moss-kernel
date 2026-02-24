use paste::paste;
use tock_registers::interfaces::{ReadWriteable, Readable};
use tock_registers::{register_bitfields, registers::InMemoryRegister};

use crate::memory::PAGE_SHIFT;
use crate::memory::address::{PA, VA};
use crate::memory::permissions::PtePermissions;
use crate::memory::region::PhysMemoryRegion;

pub trait PageTableEntry: Sized + Copy + Clone {
    fn is_valid(self) -> bool;
    fn as_raw(self) -> u64;
    fn from_raw(v: u64) -> Self;
    fn invalid() -> Self;
}

pub trait TableMapper: PageTableEntry {
    fn next_table_address(self) -> Option<PA>;
    fn new_next_table(pa: PA) -> Self;
}

pub trait PaMapper: PageTableEntry {
    fn new_map_pa(page_address: PA, memory_type: MemoryType, perms: PtePermissions) -> Self;
    fn map_shift() -> usize;
    fn could_map(region: PhysMemoryRegion, va: VA) -> bool;
    fn mapped_address(self) -> Option<PA>;
}

#[derive(Debug, Clone, Copy)]
pub enum MemoryType {
    Normal,
}

register_bitfields![u64,
    pub GenericFields [
        PRESENT  OFFSET(0) NUMBITS(1) [],
        WRITABLE OFFSET(1) NUMBITS(1) [],
        USER     OFFSET(2) NUMBITS(1) [],
        ACCESSED OFFSET(5) NUMBITS(1) [],
        DIRTY    OFFSET(6) NUMBITS(1) [],
        HUGE     OFFSET(7) NUMBITS(1) [],
        NX       OFFSET(63) NUMBITS(1) [],
        // Software defined bit for CoW (using bit 9 which is often available)
        COW      OFFSET(9) NUMBITS(1) [],
        OUTPUT_ADDR OFFSET(12) NUMBITS(36) []
    ]
];

macro_rules! define_descriptor {
    (
        $(#[$outer:meta])*
        $name:ident,
        $( table: $is_table:expr, )?
        $( map: { shift: $tbl_shift:literal }, )?
    ) => {
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        $(#[$outer])*
        pub struct $name(u64);

        impl PageTableEntry for $name {
            fn is_valid(self) -> bool { (self.0 & 1) != 0 }
            fn as_raw(self) -> u64 { self.0 }
            fn from_raw(v: u64) -> Self { Self(v) }
            fn invalid() -> Self { Self(0) }
        }

        $(
            impl TableMapper for $name {
                fn next_table_address(self) -> Option<PA> {
                    let _ = $is_table;
                    let reg = InMemoryRegister::new(self.0);
                    if reg.is_set(GenericFields::PRESENT) {
                        // For non-leaf levels, PS=0 means it's a table.
                        // PML4 always has PS=0 (reserved).
                        // PDPT/PD can have PS=1 for huge pages.
                        if stringify!($name) == "PML4Descriptor" || !reg.is_set(GenericFields::HUGE) {
                            let addr = reg.read(GenericFields::OUTPUT_ADDR);
                            return Some(PA::from_value((addr << 12) as usize));
                        }
                    }
                    None
                }

                fn new_next_table(pa: PA) -> Self {
                    let _ = $is_table;
                    let reg = InMemoryRegister::new(0);
                    reg.modify(GenericFields::PRESENT::SET +
                               GenericFields::WRITABLE::SET +
                               GenericFields::USER::SET +
                               GenericFields::OUTPUT_ADDR.val((pa.value() >> 12) as u64));
                    Self(reg.get())
                }
            }
        )?

        $(
            impl $name {
                pub fn permissions(self) -> Option<PtePermissions> {
                    if !self.is_valid() { return None; }
                    let reg = InMemoryRegister::new(self.0);
                    
                    let write = reg.is_set(GenericFields::WRITABLE);
                    let user = reg.is_set(GenericFields::USER);
                    let nx = reg.is_set(GenericFields::NX);
                    let cow = reg.is_set(GenericFields::COW);

                    Some(PtePermissions::from_raw_bits(
                        true,
                        write,
                        !nx,
                        user,
                        cow
                    ))
                }

                pub fn set_permissions(self, perms: PtePermissions) -> Self {
                    let reg = InMemoryRegister::new(self.0);
                    
                    if perms.is_write() {
                        reg.modify(GenericFields::WRITABLE::SET);
                    } else {
                        reg.modify(GenericFields::WRITABLE::CLEAR);
                    }

                    if perms.is_user() {
                        reg.modify(GenericFields::USER::SET);
                    } else {
                        reg.modify(GenericFields::USER::CLEAR);
                    }

                    if perms.is_execute() {
                        reg.modify(GenericFields::NX::CLEAR);
                    } else {
                        reg.modify(GenericFields::NX::SET);
                    }

                    if perms.is_cow() {
                        reg.modify(GenericFields::COW::SET);
                    } else {
                        reg.modify(GenericFields::COW::CLEAR);
                    }

                    Self(reg.get())
                }
            }

            impl PaMapper for $name {
                fn map_shift() -> usize { $tbl_shift }

                fn could_map(region: PhysMemoryRegion, va: VA) -> bool {
                    let is_aligned = |addr: usize| (addr & ((1 << $tbl_shift) - 1)) == 0;
                    is_aligned(region.start_address().value())
                        && is_aligned(va.value())
                        && region.size() >= (1 << $tbl_shift)
                }

                fn new_map_pa(page_address: PA, _memory_type: MemoryType, perms: PtePermissions) -> Self {
                    let reg = InMemoryRegister::new(0);
                    reg.modify(GenericFields::PRESENT::SET +
                               GenericFields::ACCESSED::SET +
                               GenericFields::OUTPUT_ADDR.val((page_address.value() >> 12) as u64));
                    
                    if $tbl_shift > 12 {
                        reg.modify(GenericFields::HUGE::SET);
                    }

                    Self(reg.get()).set_permissions(perms)
                }

                fn mapped_address(self) -> Option<PA> {
                    if !self.is_valid() { return None; }
                    let reg = InMemoryRegister::new(self.0);
                    let addr = reg.read(GenericFields::OUTPUT_ADDR);
                    Some(PA::from_value((addr << 12) as usize))
                }
            }
        )?
    };
}

define_descriptor!(
    PML4Descriptor,
    table: true,
);

define_descriptor!(
    PDPTDescriptor,
    table: true,
    map: { shift: 30 }, // 1GiB blocks
);

define_descriptor!(
    PDDescriptor,
    table: true,
    map: { shift: 21 }, // 2MiB blocks
);

define_descriptor!(
    PTDescriptor,
    map: { shift: 12 }, // 4KiB pages
);
