//! Compact repository command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, CompactArgs};

pub async fn run(cli: &Cli, args: &CompactArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    println!("Compacting repository: {}", repo_path);
    println!("Threshold: {}%", args.threshold);

    // Compact implementation

    Ok(())
}
