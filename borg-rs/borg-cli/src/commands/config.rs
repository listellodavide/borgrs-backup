//! Configuration command

use anyhow::Result;
use crate::{Cli, ConfigArgs};

pub async fn run(_cli: &Cli, args: &ConfigArgs) -> Result<()> {
    if args.show {
        println!("Current configuration:");
        println!("  BORG_REPO: {}", std::env::var("BORG_REPO").unwrap_or_default());
        println!("  BORG_PASSPHRASE: {}", 
            if std::env::var("BORG_PASSPHRASE").is_ok() { "(set)" } else { "(not set)" }
        );
        return Ok(());
    }

    match (&args.key, &args.value) {
        (Some(key), Some(value)) => {
            println!("Setting {} = {}", key, value);
        }
        (Some(key), None) => {
            println!("Getting value for: {}", key);
        }
        _ => {
            println!("Usage: borg config [--show] [key] [value]");
        }
    }

    Ok(())
}
