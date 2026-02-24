#![no_std]
#![no_main]
#![feature(used_with_arg)]
#![feature(likely_unlikely)]
#![feature(box_as_ptr)]
#![expect(internal_features)]
#![feature(core_intrinsics)]
#![feature(custom_test_frameworks)]
#![reexport_test_harness_main = "test_main"]
#![test_runner(crate::testing::test_runner)]

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use arch::{Arch, ArchImpl};
use core::panic::PanicInfo;
use drivers::{fdt_prober::get_fdt, fs::register_fs_drivers};
use fs::VFS;
use libkernel::{
    CpuOps, VirtualMemory,
    fs::{
        BlockDevice, OpenFlags, attr::FilePermissions, blk::ramdisk::RamdiskBlkDev, path::Path,
        pathbuf::PathBuf,
    },
    memory::{
        address::{PA, VA},
        region::PhysMemoryRegion,
    },
};
use log::{error, warn};
use process::ctx::UserCtx;
use sched::{
    current::current_task_shared, sched_init, spawn_kernel_work, uspc_ret::dispatch_userspace_task,
};

extern crate alloc;
extern crate moss_macros;

mod arch;
mod clock;
mod console;
mod drivers;
mod fs;
mod interrupts;
mod kernel;
mod memory;
mod process;
mod sched;
mod sync;
#[cfg(test)]
pub mod testing;

#[panic_handler]
fn on_panic(info: &PanicInfo) -> ! {
    ArchImpl::disable_interrupts();

    let panic_msg = info.message();

    if let Some(location) = info.location() {
        error!(
            "Kernel panicked at {}:{}:{}: {}",
            location.file(),
            location.line(),
            location.column(),
            panic_msg
        );
    } else {
        error!("Kernel panicked at unknown location: {panic_msg}");
    }

    ArchImpl::power_off();
}

async fn launch_init(mut opts: KOptions) {
    let init = opts
        .init
        .unwrap_or_else(|| panic!("No init specified in kernel command line"));

    let dt = get_fdt();

    let initrd_block_dev: Option<Box<dyn BlockDevice>> = if let Some(chosen) =
        dt.find_nodes("/chosen").next()
        && let Some(start_addr) = chosen
            .find_property("linux,initrd-start")
            .map(|prop| prop.u64())
        && let Some(end_addr) = chosen
            .find_property("linux,initrd-end")
            .map(|prop| prop.u64())
    {
        let region = PhysMemoryRegion::from_start_end_address(
            PA::from_value(start_addr as _),
            PA::from_value(end_addr as _),
        );

        Some(Box::new(
            RamdiskBlkDev::new(
                region,
                VA::from_value(0xffff_9800_0000_0000),
                &mut *ArchImpl::kern_address_space().lock_save_irq(),
            )
            .unwrap(),
        ))
    } else {
        None
    };

    let root_fs = opts
        .root_fs
        .unwrap_or_else(|| panic!("No root FS driver specified in kernel command line"));

    VFS.mount_root(&root_fs, initrd_block_dev)
        .await
        .unwrap_or_else(|e| panic!("Failed to mount root FS: {}", e));

    // Process all automounts.
    for (path, fs) in opts.automounts.iter() {
        let mount_point = VFS
            .resolve_path_absolute(path, VFS.root_inode())
            .await
            .unwrap_or_else(|e| panic!("Could not find automount path: {}. {e}", path.as_str()));

        VFS.mount(mount_point, fs, None)
            .await
            .unwrap_or_else(|e| panic!("Automount failed: {e}"));
    }

    let inode = VFS
        .resolve_path_absolute(&init, VFS.root_inode())
        .await
        .expect("Unable to find init");

    let task = current_task_shared();

    // Ensure that the exec() call applies to init.
    assert!(task.process.tgid.is_init());

    // Now that the root fs has been mounted, set the real root inode as the
    // cwd and root.
    *task.cwd.lock_save_irq() = (VFS.root_inode(), PathBuf::from("/"));
    *task.root.lock_save_irq() = (VFS.root_inode(), PathBuf::from("/"));

    let console = VFS
        .open(
            Path::new("/dev/console"),
            OpenFlags::O_RDWR,
            VFS.root_inode(),
            FilePermissions::empty(),
            &task,
        )
        .await
        .expect("Could not open console for init process");

    {
        let mut fd_table = task.fd_table.lock_save_irq();

        // stdin, stdout, stderr
        fd_table
            .insert(console.clone())
            .expect("Could not clone FD");
        fd_table
            .insert(console.clone())
            .expect("Could not clone FD");
        fd_table
            .insert(console.clone())
            .expect("Could not clone FD");
    }

    #[cfg(test)]
    test_main();

    drop(task);

    let mut init_args = vec![init.as_str().to_string()];

    init_args.append(&mut opts.init_args);

    process::exec::kernel_exec(init.as_path(), inode, init_args, vec![])
        .await
        .expect("Could not launch init process");
}

struct KOptions {
    init: Option<PathBuf>,
    root_fs: Option<String>,
    automounts: Vec<(PathBuf, String)>,
    init_args: Vec<String>,
}

fn parse_args(args: &str) -> KOptions {
    let mut kopts = KOptions {
        init: None,
        root_fs: None,
        automounts: Vec::new(),
        init_args: Vec::new(),
    };

    let mut iter = args.split_whitespace().peekable();
    while let Some(token) = iter.next() {
        if !token.starts_with("--") {
            continue;
        }

        // support `--key=value` or `--key value`
        let mut parts = token[2..].splitn(2, '=');
        let key = parts.next().unwrap();
        let value = parts.next().map(|s| s.to_string()).or_else(|| {
            // try to get next token as value if it doesn't start with `--`
            if let Some(next) = iter.peek() {
                if !next.starts_with("--") {
                    return Some(iter.next().unwrap().to_string());
                }
            }
            None
        });

        match key {
            "init" => {
                if let Some(v) = value {
                    kopts.init = Some(PathBuf::from(v));
                }
            }
            "init-arg" => {
                if let Some(v) = value {
                    kopts.init_args.push(v);
                }
            }
            "rootfs" => {
                if let Some(v) = value {
                    kopts.root_fs = Some(v);
                }
            }
            "automount" => {
                if let Some(v) = value {
                    let mut split = v.split(',');
                    if let (Some(path), Some(fs)) = (split.next(), split.next()) {
                        kopts.automounts.push((PathBuf::from(path), fs.to_string()));
                    }
                }
            }
            other => {
                warn!("Unknown option {}", other);
            }
        }
    }

    kopts
}

pub fn kmain(args: String, ctx_frame: *mut UserCtx) {
    sched_init();

    register_fs_drivers();

    let kopts = parse_args(&args);

    spawn_kernel_work(launch_init(kopts));

    dispatch_userspace_task(ctx_frame);
}
