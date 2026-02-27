use crate::{
    arch::{ArchImpl, x86_64::exceptions::ExceptionState},
    memory::{PageOffsetTranslator, page::ClaimedPage},
    process::owned::OwnedTask,
};
use core::arch::global_asm;
use libkernel::{
    UserAddressSpace, VirtualMemory,
    memory::{
        address::VA,
        permissions::PtePermissions,
        proc_vm::vmarea::{VMAPermissions, VMArea, VMAreaKind},
        region::VirtMemoryRegion,
    },
};

global_asm!(include_str!("idle.s"));

pub fn create_idle_task() -> OwnedTask {
    let code_page = ClaimedPage::alloc_zeroed().unwrap().leak();
    let code_addr = VA::from_value(0xd00d0000);

    unsafe extern "C" {
        static __idle_start: u8;
        static __idle_end: u8;
    }

    let idle_start_ptr = unsafe { &__idle_start } as *const u8;
    let idle_end_ptr = unsafe { &__idle_end } as *const u8;
    let code_sz = idle_end_ptr.addr() - idle_start_ptr.addr();

    unsafe {
        idle_start_ptr.copy_to(
            code_page
                .pa()
                .to_va::<PageOffsetTranslator>()
                .cast::<u8>()
                .as_ptr_mut(),
            code_sz,
        );
    };

    let mut addr_space = <ArchImpl as VirtualMemory>::ProcessAddressSpace::new().unwrap();

    addr_space
        .map_page(code_page, code_addr, PtePermissions::rx(true))
        .unwrap();

    let ctx = ExceptionState {
        rax: 0, rcx: 0, rdx: 0, rbx: 0, rbp: 0, rsi: 0, rdi: 0,
        r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
        vector: 0,
        error_code: 0,
        rip: code_addr.value() as _,
        cs: super::super::USER_CS,
        rflags: 0x202, // IF
        rsp: 0,
        ss: super::super::USER_SS,
    };

    let code_map = VMArea::new(
        VirtMemoryRegion::new(code_addr, code_sz),
        VMAreaKind::Anon,
        VMAPermissions::rx(),
    );

    OwnedTask::create_idle_task(addr_space, ctx, code_map)
}
