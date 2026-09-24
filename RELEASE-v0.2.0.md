# x86_64 Port Release Notes

## v0.2.0 — x86_64 Architecture Support

Complete x86_64 kernel port with boot verification under QEMU.

### What's Included

#### Phases 0–5
- **Phase 0**: Build infrastructure, conditional compilation, architecture skeleton
- **Phase 1**: Boot process (Multiboot2, GDT/TSS, linker script)
- **Phase 2**: Memory management (PML4/PDPT/PD/PT, kernel/process address spaces)
- **Phase 3**: Exception handling (IDT, ISR stubs, fault handlers, LAPIC/IOAPIC)
- **Phase 4**: Process management (context switch, idle task, signals, VDSO, ptrace)
- **Phase 5**: Driver porting (16550 UART, APIC, HPET timer, LAPIC timer)

#### Boot Fixes
5 critical bugs preventing x86_64 QEMU boot:
1. Wrong Multiboot2 register magic
2. Boot page tables only mapped 4 MiB
3. Higher-half PDPT index was wrong
4. PVH ELF note corrupted LOAD layout
5. Early serial diagnostic clobbered EAX

#### CI/CD
- x86_64 bare-metal build + clippy
- AArch64 cross-compile check
- QEMU Multiboot2 boot test
- libkernel unit tests

### Boot Verification
```
SeaBIOS → iPXE → GRUB → "Booting `moss kernel'"
  → SLCEXIT=0 (S=_start, L=long mode, C=arch_init_stage1, clean exit)
```

### Register States
- RIP = 0xFFFFFFFF80000000
- RSP = 0x100000
- CS = 0x08, DS = 0x10

### Bootable ISO
Download `moss-v0.2.0-x86_64.iso` to boot via QEMU:
```bash
qemu-system-x86_64 -cdrom moss-v0.2.0-x86_64.iso -nographic -serial stdio -m 2G
```
