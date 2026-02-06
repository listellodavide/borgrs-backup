//! Unmount FUSE filesystem

use anyhow::Result;
use crate::{Cli, UmountArgs};

pub async fn run(_cli: &Cli, args: &UmountArgs) -> Result<()> {
    println!("Unmounting {:?}", args.mountpoint);

    // FUSE unmount implementation
    #[cfg(unix)]
    {
        use std::process::Command;
        use anyhow::Context;
        let status = Command::new("fusermount")
            .arg("-u")
            .arg(&args.mountpoint)
            .status()
            .context("Failed to execute fusermount")?;

        if !status.success() {
            anyhow::bail!("Failed to unmount");
        }
    }

    Ok(())
}
