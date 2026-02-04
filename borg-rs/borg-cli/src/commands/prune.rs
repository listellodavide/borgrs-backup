//! Prune archives command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, PruneArgs};

pub async fn run(cli: &Cli, args: &PruneArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    println!("Pruning archives with retention policy:");
    if let Some(h) = args.keep_hourly { println!("  keep-hourly: {}", h); }
    if let Some(d) = args.keep_daily { println!("  keep-daily: {}", d); }
    if let Some(w) = args.keep_weekly { println!("  keep-weekly: {}", w); }
    if let Some(m) = args.keep_monthly { println!("  keep-monthly: {}", m); }
    if let Some(y) = args.keep_yearly { println!("  keep-yearly: {}", y); }

    Ok(())
}
