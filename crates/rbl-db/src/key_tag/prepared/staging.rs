//! Detect exchange support using only our staged copy and an empty temporary.
//! A supported exchange needs one full copy, even at peak. Only fallback needs
//! a second full copy; that copy and all verification remain outside the gate.
use super::{exchange, keep_created, keep_owner, Prepared, Result};
use std::{fs, io};

impl Prepared {
    pub(super) fn stage_backup(
        &mut self,
        authorize: &impl Fn() -> io::Result<()>,
        forced: Option<io::ErrorKind>,
    ) -> Result<()> {
        authorize()?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.backup)?;
        authorize()?;
        let probe = match forced {
            Some(kind) => Err(io::Error::new(kind, "forced unsupported exchange")),
            None => exchange(&self.temp, &self.backup),
        };
        match probe {
            Ok(()) => {
                // The full copy is now at backup, the empty inode at temp.
                // Move it back without another audio copy; original untouched.
                authorize()?;
                fs::rename(&self.backup, &self.temp)?;
            }
            Err(why) if unsupported(&why) => {
                authorize()?;
                fs::copy(&self.temp, &self.backup)?;
                let backup = fs::OpenOptions::new().write(true).open(&self.backup)?;
                keep_created(&backup, &self.original);
                keep_owner(&backup, &self.original)?;
                backup.sync_all()?;
                self.backup_stamp = Some(backup.metadata()?);
            }
            Err(why) => return Err(why.into()),
        }
        Ok(())
    }
}

fn unsupported(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
    ) {
        return true;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if error.raw_os_error() == Some(rustix::io::Errno::OPNOTSUPP.raw_os_error()) {
        return true;
    }
    false
}
