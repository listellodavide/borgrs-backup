//! Archive extraction command

use anyhow::Result;
use super::{get_repo_path, open_repository};
use crate::{Cli, ExtractArgs};

pub async fn run(cli: &Cli, args: &ExtractArgs) -> Result<()> {
    let repo_path = get_repo_path(cli)?;
    let _repo = open_repository(&repo_path).await?;

    // Implementation would extract files from archive
    println!("Extracting from archive: {}", args.archive);

    Ok(())
}
