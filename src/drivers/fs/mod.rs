use alloc::sync::Arc;
use dev::DevFsDriver;
use ext4::Ext4FsDriver;
use fat32::Fat32FsDriver;
use tmpfs::TmpFsDriver;
use proc::ProcFsDriver;
use sys::SysFsDriver;
use cgroup::CgroupFsDriver;

use super::DM;

pub mod cgroup;
pub mod dev;
pub mod ext4;
pub mod fat32;
pub mod proc;
pub mod sys;
pub mod tmpfs;

pub fn register_fs_drivers() {
    let mut dm = DM.lock_save_irq();

    dm.insert_driver(Arc::new(Ext4FsDriver::new()));
    dm.insert_driver(Arc::new(Fat32FsDriver::new()));
    dm.insert_driver(Arc::new(DevFsDriver::new()));
    dm.insert_driver(Arc::new(TmpFsDriver::new()));
    dm.insert_driver(Arc::new(ProcFsDriver));
    dm.insert_driver(Arc::new(SysFsDriver));
    dm.insert_driver(Arc::new(CgroupFsDriver));
}
