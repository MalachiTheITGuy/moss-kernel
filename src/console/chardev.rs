use crate::{
    drivers::{
        CharDriver, DM, DriverManager, OpenableDevice, ReservedMajors, fs::dev::devfs,
        init::PlatformBus,
    },
    fs::open_file::OpenFile,
    kernel_driver,
    process::fd_table::Fd,
    sched::current::current_task,
};
use alloc::{string::ToString, sync::Arc};
use libkernel::{
    driver::CharDevDescriptor,
    error::{FsError, Result},
    fs::{OpenFlags, attr::FilePermissions},
};

use super::CONSOLE;

struct TtyDev {}

impl OpenableDevice for TtyDev {
    fn open(&self, _args: OpenFlags) -> Result<Arc<OpenFile>> {
        let task = current_task();

        // TODO: This should really open the controlling terminal of the
        // session.
        Ok(task
            .fd_table
            .lock_save_irq()
            .get(Fd(0))
            .ok_or(FsError::NoDevice)?)
    }
}

struct ConsoleDev {}

impl OpenableDevice for ConsoleDev {
    fn open(&self, flags: OpenFlags) -> Result<Arc<OpenFile>> {
        let char_dev_desc = {
            let state = CONSOLE.lock_save_irq();
            match *state {
                super::ConsoleState::Buffered => {
                    log::debug!("ConsoleDev::open - console still buffered");
                    return Err(FsError::NoDevice.into());
                }
                super::ConsoleState::Device(_, char_dev_descriptor) => char_dev_descriptor,
            }
        };
        log::debug!(
            "ConsoleDev::open called (flags={:?}) using descriptor {:?}",
            flags,
            char_dev_desc
        );

        // Lookup the underlying char driver while holding DM lock, but drop it
        // before performing the actual open call so we don't hold the lock during
        // potentially complex device initialization (which may itself acquire
        // other locks).
        let char_driver = {
            let mut dm_lock = DM.lock_save_irq();
            let drv_opt = dm_lock.find_char_driver(char_dev_desc.major);
            drv_opt.map(|arc| arc.clone())
        };

        let char_driver = match char_driver {
            Some(d) => {
                log::debug!("found char driver for major {}", char_dev_desc.major);
                d
            }
            None => {
                log::warn!("no char driver for major {}", char_dev_desc.major);
                return Err(FsError::NoDevice.into());
            }
        };

        if let Some(dev) = char_driver.get_device(char_dev_desc.minor) {
            log::debug!("found device for minor {}", char_dev_desc.minor);
            dev.open(flags)
        } else {
            log::warn!("no device present for minor {}", char_dev_desc.minor);
            Err(FsError::NoDevice.into())
        }
    }
}

struct ConsoleCharDev {
    tty_dev: Arc<dyn OpenableDevice>,
    console_dev: Arc<dyn OpenableDevice>,
}

impl ConsoleCharDev {
    pub fn new() -> Result<Self> {
        devfs().mknod(
            "console".to_string(),
            CharDevDescriptor {
                major: ReservedMajors::Console as _,
                minor: 1,
            },
            FilePermissions::from_bits_retain(0o600),
        )?;

        devfs().mknod(
            "tty".to_string(),
            CharDevDescriptor {
                major: ReservedMajors::Console as _,
                minor: 0,
            },
            FilePermissions::from_bits_retain(0o600),
        )?;

        Ok(Self {
            tty_dev: Arc::new(TtyDev {}),
            console_dev: Arc::new(ConsoleDev {}),
        })
    }
}

impl CharDriver for ConsoleCharDev {
    fn get_device(&self, minor: u64) -> Option<Arc<dyn OpenableDevice>> {
        match minor {
            0 => Some(self.tty_dev.clone()),
            1 => Some(self.console_dev.clone()),
            _ => None,
        }
    }
}

pub fn console_chardev_init(_bus: &mut PlatformBus, dm: &mut DriverManager) -> Result<()> {
    let ccd = ConsoleCharDev::new()?;

    dm.register_char_driver(ReservedMajors::Console as _, Arc::new(ccd))?;

    Ok(())
}

kernel_driver!(console_chardev_init);
